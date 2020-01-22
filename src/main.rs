use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::iter::FromIterator;
use std::ops::Range;
use std::time::Instant;
use std::{cmp, fmt, iter::once};

use big_s::S;
use intervaltree::IntervalTree;
use itertools::{EitherOrBoth, merge_join_by};
use maplit::hashmap;
use query_words_mapper::QueryWordsMapper;
use rand::{Rng, SeedableRng, rngs::StdRng};
use sdset::{Set, SetBuf, SetOperation};
use slice_group_by::StrGroupBy;

mod query_words_mapper;

#[derive(Clone, PartialEq, Eq, Hash)]
enum Operation {
    And(Vec<Operation>),
    Or(Vec<Operation>),
    Query(Query),
}

impl fmt::Debug for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn pprint_tree(f: &mut fmt::Formatter<'_>, op: &Operation, depth: usize) -> fmt::Result {
            match op {
                Operation::And(children) => {
                    writeln!(f, "{:1$}AND", "", depth * 2)?;
                    children.iter().try_for_each(|c| pprint_tree(f, c, depth + 1))
                },
                Operation::Or(children) => {
                    writeln!(f, "{:1$}OR", "", depth * 2)?;
                    children.iter().try_for_each(|c| pprint_tree(f, c, depth + 1))
                },
                Operation::Query(query) => writeln!(f, "{:2$}{:?}", "", query, depth * 2),
            }
        }

        pprint_tree(f, self, 0)
    }
}

impl Operation {
    fn tolerant(id: QueryId, prefix: bool, s: &str) -> Operation {
        Operation::Query(Query { id, prefix, kind: QueryKind::Tolerant(s.to_string()) })
    }

    fn exact(id: QueryId, prefix: bool, s: &str) -> Operation {
        Operation::Query(Query { id, prefix, kind: QueryKind::Exact(s.to_string()) })
    }

    fn phrase2(id: QueryId, prefix: bool, (left, right): (&str, &str)) -> Operation {
        Operation::Query(Query { id, prefix, kind: QueryKind::Phrase(vec![left.to_owned(), right.to_owned()]) })
    }
}

type QueryId = usize;

#[derive(Clone, Eq)]
struct Query {
    id: QueryId,
    prefix: bool,
    kind: QueryKind,
}

impl PartialEq for Query {
    fn eq(&self, other: &Self) -> bool {
        self.prefix == other.prefix && self.kind == other.kind
    }
}

impl Hash for Query {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.prefix.hash(state);
        self.kind.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum QueryKind {
    Tolerant(String),
    Exact(String),
    Phrase(Vec<String>),
}

impl fmt::Debug for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Query { id, prefix, kind } = self;
        let prefix = if *prefix { String::from("Prefix") } else { String::default() };
        match kind {
            QueryKind::Exact(word) => {
                f.debug_struct(&(prefix + "Exact")).field("id", &id).field("word", &word).finish()
            },
            QueryKind::Tolerant(word) => {
                f.debug_struct(&(prefix + "Tolerant")).field("id", &id).field("word", &word).finish()
            },
            QueryKind::Phrase(words) => {
                f.debug_struct(&(prefix + "Phrase")).field("id", &id).field("words", &words).finish()
            },
        }
    }
}

type DocId = u16;
type Position = u8;

#[derive(Debug, Default)]
struct PostingsList {
    docids: SetBuf<DocId>,
    matches: SetBuf<(DocId, Position)>,
}

#[derive(Debug, Default)]
struct Context {
    synonyms: HashMap<Vec<String>, Vec<Vec<String>>>,
    postings: HashMap<String, PostingsList>,
}

fn split_best_frequency<'a>(ctx: &Context, word: &'a str) -> Option<(&'a str, &'a str)> {
    let chars = word.char_indices().skip(1);
    let mut best = None;

    for (i, _) in chars {
        let (left, right) = word.split_at(i);

        let left_freq = ctx.postings.get(left).map(|b| b.docids.len()).unwrap_or(0);
        let right_freq = ctx.postings.get(right).map(|b| b.docids.len()).unwrap_or(0);

        let min_freq = cmp::min(left_freq, right_freq);
        if min_freq != 0 && best.map_or(true, |(old, _, _)| min_freq > old) {
            best = Some((min_freq, left, right));
        }
    }

    best.map(|(_, l, r)| (l, r))
}

fn fetch_synonyms<S: AsRef<str>>(ctx: &Context, words: &[S]) -> Vec<Vec<String>> {
    let words: Vec<_> = words.iter().map(|s| s.as_ref().to_owned()).collect(); // TODO ugly
    ctx.synonyms.get(&words).cloned().unwrap_or_default()
}

fn is_last<I: IntoIterator>(iter: I) -> impl Iterator<Item=(bool, I::Item)> {
    let mut iter = iter.into_iter().peekable();
    core::iter::from_fn(move || {
        iter.next().map(|item| (iter.peek().is_none(), item))
    })
}

fn create_operation<I, F>(iter: I, f: F) -> Operation
where I: IntoIterator<Item=Operation>,
      F: Fn(Vec<Operation>) -> Operation,
{
    let mut iter = iter.into_iter();
    match (iter.next(), iter.next()) {
        (Some(first), None) => first,
        (first, second) => f(first.into_iter().chain(second).chain(iter).collect()),
    }
}

const MAX_NGRAM: usize = 3;

fn create_query_tree(ctx: &Context, query: &str) -> (Operation, HashMap<QueryId, Range<usize>>) {
    let query = query.to_lowercase();
    let words = query.linear_group_by_key(char::is_whitespace).map(ToOwned::to_owned);
    let words: Vec<_> = words.filter(|s| !s.contains(char::is_whitespace)).enumerate().collect();

    let mut mapper = QueryWordsMapper::new(words.iter().map(|(_, w)| w));

    fn create_inner(ctx: &Context, mapper: &mut QueryWordsMapper, words: &[(usize, String)]) -> Vec<Operation> {
        let mut alts = Vec::new();

        for ngram in 1..=MAX_NGRAM {
            if let Some(group) = words.get(..ngram) {
                let mut group_ops = Vec::new();

                let tail = &words[ngram..];
                let is_last = tail.is_empty();

                let mut group_alts = Vec::new();
                match group {
                    [(id, word)] => {
                        let mut idgen = ((id + 1) * 100)..;
                        let range = (*id)..id+1;

                        let phrase = split_best_frequency(ctx, word).map(|ws| {
                            let id = idgen.next().unwrap();
                            idgen.next().unwrap();
                            mapper.declare(range.clone(), id, &[ws.0, ws.1]);
                            Operation::phrase2(id, is_last, ws)
                        });

                        let synonyms = fetch_synonyms(ctx, &[word]).into_iter().map(|alts| {
                            let id = idgen.next().unwrap();
                            mapper.declare(range.clone(), id, &alts);

                            let mut idgen = once(id).chain(&mut idgen);
                            let iter = alts.into_iter().map(|w| {
                                let id = idgen.next().unwrap();
                                Operation::exact(id, false, &w)
                            });

                            create_operation(iter, Operation::And)
                        });

                        let original = Operation::tolerant(*id, is_last, word);

                        group_alts.push(original);
                        group_alts.extend(synonyms.chain(phrase));
                    },
                    words => {
                        let id = words[0].0;
                        let mut idgen = ((id + 1) * 100_usize.pow(ngram as u32))..;
                        let range = id..id+ngram;

                        let words: Vec<_> = words.iter().map(|(_, s)| s.as_str()).collect();

                        for synonym in fetch_synonyms(ctx, &words) {
                            let id = idgen.next().unwrap();
                            mapper.declare(range.clone(), id, &synonym);

                            let mut idgen = once(id).chain(&mut idgen);
                            let synonym = synonym.into_iter().map(|s| {
                                let id = idgen.next().unwrap();
                                Operation::exact(id, false, &s)
                            });
                            group_alts.push(create_operation(synonym, Operation::And));
                        }

                        let id = idgen.next().unwrap();
                        let concat = words.concat();
                        mapper.declare(range.clone(), id, &[&concat]);
                        group_alts.push(Operation::exact(id, is_last, &concat));
                    }
                }

                group_ops.push(create_operation(group_alts, Operation::Or));

                if !tail.is_empty() {
                    let tail_ops = create_inner(ctx, mapper, tail);
                    group_ops.push(create_operation(tail_ops, Operation::Or));
                }

                alts.push(create_operation(group_ops, Operation::And));
            }
        }

        alts
    }

    let operation = Operation::Or(create_inner(ctx, &mut mapper, &words));
    let mapping = mapper.mapping();

    (operation, mapping)
}

struct QueryResult<'q, 'c> {
    docids: Cow<'c, Set<DocId>>,
    queries: HashMap<&'q Query, Cow<'c, Set<(DocId, Position)>>>,
}

type Postings<'q, 'c> = HashMap<&'q Query, Cow<'c, Set<(DocId, Position)>>>;
type Cache<'o, 'c> = HashMap<&'o Operation, Cow<'c, Set<DocId>>>;

fn traverse_query_tree<'a, 'c>(ctx: &'c Context, tree: &'a Operation) -> QueryResult<'a, 'c> {
    fn execute_and<'o, 'c>(
        ctx: &'c Context,
        cache: &mut Cache<'o, 'c>,
        postings: &mut Postings<'o, 'c>,
        depth: usize,
        operations: &'o [Operation],
    ) -> Cow<'c, Set<DocId>>
    {
        println!("{:1$}AND", "", depth * 2);

        let before = Instant::now();
        let mut results = Vec::new();

        for op in operations {
            if cache.get(op).is_none() {
                let docids = match op {
                    Operation::And(ops) => execute_and(ctx, cache, postings, depth + 1, &ops),
                    Operation::Or(ops) => execute_or(ctx, cache, postings, depth + 1, &ops),
                    Operation::Query(query) => execute_query(ctx, postings, depth + 1, &query),
                };
                cache.insert(op, docids);
            }
        }

        for op in operations {
            if let Some(docids) = cache.get(op) {
                results.push(docids.as_ref());
            }
        }

        let op = sdset::multi::Intersection::new(results);
        let docids = op.into_set_buf();
        let docids: Cow<Set<_>> = Cow::Owned(docids);

        println!("{:3$}--- AND fetched {} documents in {:.02?}", "", docids.len(), before.elapsed(), depth * 2);

        docids
    }

    fn execute_or<'o, 'c>(
        ctx: &'c Context,
        cache: &mut Cache<'o, 'c>,
        postings: &mut Postings<'o, 'c>,
        depth: usize,
        operations: &'o [Operation],
    ) -> Cow<'c, Set<DocId>>
    {
        println!("{:1$}OR", "", depth * 2);

        let before = Instant::now();
        let mut ids = Vec::new();

        for op in operations {
            let docids = match cache.get(op) {
                Some(docids) => docids,
                None => {
                    let docids = match op {
                        Operation::And(ops) => execute_and(ctx, cache, postings, depth + 1, &ops),
                        Operation::Or(ops) => execute_or(ctx, cache, postings, depth + 1, &ops),
                        Operation::Query(query) => execute_query(ctx, postings, depth + 1, &query),
                    };
                    cache.entry(op).or_insert(docids)
                }
            };

            ids.extend(docids.as_ref());
        }

        let docids = SetBuf::from_dirty(ids);
        let docids: Cow<Set<_>> = Cow::Owned(docids);

        println!("{:3$}--- OR fetched {} documents in {:.02?}", "", docids.len(), before.elapsed(), depth * 2);

        docids
    }

    fn execute_query<'o, 'c>(
        ctx: &'c Context,
        postings: &mut Postings<'o, 'c>,
        depth: usize,
        query: &'o Query,
    ) -> Cow<'c, Set<DocId>>
    {
        let before = Instant::now();

        let Query { id, prefix, kind } = query;
        let (docids, matches) = match kind {
              QueryKind::Tolerant(word) | QueryKind::Exact(word) => {
                if let Some(PostingsList { docids, matches }) = ctx.postings.get(word) {
                    (Cow::Borrowed(docids.as_set()), Cow::Borrowed(matches.as_set()))
                } else {
                    (Cow::default(), Cow::default())
                }
            },
            QueryKind::Phrase(words) => {
                if let [first, second] = words.as_slice() {
                    let default = SetBuf::default();
                    let first = ctx.postings.get(first).map(|pl| &pl.matches).unwrap_or(&default);
                    let second = ctx.postings.get(second).map(|pl| &pl.matches).unwrap_or(&default);

                    let iter = merge_join_by(first.as_slice(), second.as_slice(), |a, b| {
                        (a.0, (a.1 as u32) + 1).cmp(&(b.0, b.1 as u32))
                    });

                    let matches: Vec<_> = iter
                        .filter_map(EitherOrBoth::both)
                        .flat_map(|(a, b)| once(*a).chain(Some(*b)))
                        .collect();

                    let mut docids: Vec<_> = matches.iter().map(|m| m.0).collect();
                    docids.dedup();

                    println!("{:2$}matches {:?}", "", matches, depth * 2);

                    (Cow::Owned(SetBuf::new(docids).unwrap()), Cow::Owned(SetBuf::new(matches).unwrap()))
                } else {
                    println!("{:2$}{:?} skipped", "", words, depth * 2);
                    (Cow::default(), Cow::default())
                }
            },
        };

        println!("{:4$}{:?} fetched {:?} documents in {:.02?}", "", query, docids.len(), before.elapsed(), depth * 2);

        postings.insert(query, matches);
        docids
    }

    let mut cache = Cache::new();
    let mut postings = Postings::new();

    let docids = match tree {
        Operation::And(operations) => execute_and(ctx, &mut cache, &mut postings, 0, &operations),
        Operation::Or(operations) => execute_or(ctx, &mut cache, &mut postings, 0, &operations),
        Operation::Query(query) => execute_query(ctx, &mut postings, 0, &query),
    };

    QueryResult { docids, queries: postings }
}

fn random_postings<R: Rng>(rng: &mut R, len: usize) -> PostingsList {
    let mut values = BTreeSet::new();
    while values.len() != len {
        values.insert(rng.gen());
    }

    let docids = values.iter().copied().collect();
    let docids = SetBuf::new(docids).unwrap();

    let matches = docids.iter().flat_map(|id| -> Vec<(DocId, Position)> {
        let mut matches = BTreeSet::new();
        let len = rng.gen_range(1, 10);
        while matches.len() != len {
            matches.insert(rng.gen());
        }
        matches.into_iter().map(|p| (*id, p)).collect()
    }).collect();

    PostingsList { docids, matches: SetBuf::new(matches).unwrap() }
}

fn main() {
    let mut rng = StdRng::seed_from_u64(102);
    let rng = &mut rng;

    let context = Context {
        synonyms: hashmap!{
            vec![S("hello")] => vec![
                vec![S("hi")],
                vec![S("good"), S("morning")],
            ],
            vec![S("world")] => vec![
                vec![S("earth")],
                vec![S("nature")],
            ],
            vec![S("hello"), S("world")] => vec![
                vec![S("bonjour"), S("monde")],
            ],

            // new york city
            vec![S("nyc")] => vec![
                vec![S("new"), S("york")],
                vec![S("new"), S("york"), S("city")],
            ],
            vec![S("new"), S("york")] => vec![
                vec![S("nyc")],
                vec![S("new"), S("york"), S("city")],
            ],
            vec![S("new"), S("york"), S("city")] => vec![
                vec![S("nyc")],
                vec![S("new"), S("york")],
            ],
        },
        postings: hashmap!{
            S("hello")      => random_postings(rng,   1500),
            S("helloworld") => random_postings(rng,    100),
            S("hi")         => random_postings(rng,   4000),
            S("hell")       => random_postings(rng,   2500),
            S("o")          => random_postings(rng,    400),
            S("worl")       => random_postings(rng,   1400),
            S("world")      => random_postings(rng, 15_000),
            S("earth")      => random_postings(rng,   8000),
            S("2020")       => random_postings(rng,    100),
            S("2019")       => random_postings(rng,    500),
            S("is")         => random_postings(rng, 50_000),
            S("this")       => random_postings(rng, 50_000),
            S("good")       => random_postings(rng,   1250),
            S("morning")    => random_postings(rng,    125),
        },
    };

    let query = std::env::args().nth(1).unwrap_or(S("hello world"));
    let (query_tree, mapping) = create_query_tree(&context, &query);

    println!("{:?}", query_tree);
    println!("{:#?}", BTreeMap::from_iter(mapping));

    println!("---------------------------------\n");

    let QueryResult { docids, queries } = traverse_query_tree(&context, &query_tree);
    println!("found {} documents", docids.len());
    println!("number of postings {:?}", queries.len());

    let before = Instant::now();
    for (query, matches) in queries {
        let op = sdset::duo::IntersectionByKey::new(&matches, &docids, |m| m.0, Clone::clone);
        let buf: SetBuf<(u16, u8)> = op.into_set_buf();
        if !buf.is_empty() {
            println!("{:?} gives {} matches", query, buf.len());
        }
    }

    println!("matches cleaned in {:.02?}", before.elapsed());
}
