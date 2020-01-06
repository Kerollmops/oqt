use std::borrow::Cow;
use std::collections::{HashMap, BTreeSet};
use std::time::Instant;
use std::{cmp, fmt, iter::once};

use big_s::S;
use maplit::hashmap;
use rand::{Rng, SeedableRng, rngs::StdRng};
use sdset::{Set, SetBuf, SetOperation};
use slice_group_by::{StrGroupBy, GroupBy};
use itertools::{EitherOrBoth, merge_join_by};

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

type QueryId = usize;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Query {
    Tolerant(QueryId, String),
    Exact(QueryId, String),
    Prefix(QueryId, String),
    Phrase(QueryId, Vec<String>),
}

impl Query {
    fn tolerant(id: QueryId, s: &str) -> Query {
        Query::Tolerant(id, s.to_string())
    }

    fn prefix(id: QueryId, s: &str) -> Query {
        Query::Prefix(id, s.to_string())
    }

    fn phrase2(id: QueryId, (left, right): (&str, &str)) -> Query {
        Query::Phrase(id, vec![left.to_owned(), right.to_owned()])
    }
}

type DocId = u16;
type Position = u8;

#[derive(Debug, Default)]
struct Context {
    synonyms: HashMap<String, Vec<Vec<String>>>,
    postings: HashMap<String, SetBuf<(DocId, Position)>>,
}

fn split_best_frequency<'a>(ctx: &Context, word: &'a str) -> Option<(&'a str, &'a str)> {
    let chars = word.char_indices().skip(1);
    let mut best = None;

    for (i, _) in chars {
        let (left, right) = word.split_at(i);

        let left_freq = ctx.postings.get(left).map(|b| b.len()).unwrap_or(0);
        let right_freq = ctx.postings.get(right).map(|b| b.len()).unwrap_or(0);

        let min_freq = cmp::min(left_freq, right_freq);
        if min_freq != 0 && best.map_or(true, |(old, _, _)| min_freq > old) {
            best = Some((min_freq, left, right));
        }
    }

    best.map(|(_, l, r)| (l, r))
}

fn synonyms(ctx: &Context, word: &str) -> Vec<Vec<String>> {
    ctx.synonyms.get(word).cloned().unwrap_or_default()
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

fn create_query_tree(ctx: &Context, query: &str) -> Operation {
    let query = query.to_lowercase();

    let words = query.linear_group_by_key(char::is_whitespace).map(ToOwned::to_owned);
    let words = words.filter(|s| !s.contains(char::is_whitespace)).enumerate();
    let words: Vec<_> = words.collect();

    let mut ngrams = Vec::new();
    for ngram in 1..=MAX_NGRAM {
        let ngiter = words.windows(ngram).enumerate().map(|(i, g)| {
            let before = words[..i].windows(1);
            let after = words[i + ngram..].windows(1);
            before.chain(Some(g)).chain(after)
        });

        for group in ngiter {
            let mut ops = Vec::new();

            for (is_last, words) in is_last(group) {
                let mut alts = Vec::new();
                match words {
                    [(id, word)] => {
                        let phrase = split_best_frequency(ctx, word).map(|ws| Query::phrase2(*id, ws)).map(Operation::Query);
                        let synonyms = synonyms(ctx, word).into_iter().map(|alts| {
                            let iter = alts.into_iter().map(|w| Query::Exact(*id, w)).map(Operation::Query);
                            create_operation(iter, Operation::And)
                        });

                        let original = if is_last {
                            Query::prefix(*id, word)
                        } else {
                            Query::tolerant(*id, word)
                        };

                        let mut alternatives: Vec<_> = synonyms.chain(phrase).collect();

                        if !alternatives.is_empty() {
                            alts.push(Operation::Query(original));
                            alts.append(&mut alternatives);
                        } else {
                            alts.push(Operation::Query(original));
                        }
                    },
                    words => {
                        let id = words[0].0;
                        let concat = words.iter().map(|(_, s)| s.as_str()).collect();
                        alts.push(Operation::Query(Query::Exact(id, concat)));
                    }
                }

                ops.push(create_operation(alts, Operation::Or));
            }

            ngrams.push(create_operation(ops, Operation::And));
            if ngram == 1 { break }
        }
    }

    Operation::Or(ngrams)
}

struct QueryResult<'q, 'c> {
    docids: SetBuf<DocId>,
    // TODO: use an HashSet with an enum with the
    //       corresponding matches for every word
    queries: HashMap<&'q Query, Cow<'c, Set<(DocId, Position)>>>,
}

fn traverse_query_tree<'a, 'c>(ctx: &'c Context, tree: &'a Operation) -> QueryResult<'a, 'c> {

    fn execute_and<'a, 'c>(ctx: &'c Context, depth: usize, operations: &'a [Operation]) -> QueryResult<'a, 'c> {
        println!("{:1$}AND", "", depth * 2);

        let before = Instant::now();
        let mut queries = HashMap::new();
        let mut results = Vec::new();

        for op in operations {
            let result = match op {
                Operation::And(operations) => execute_and(ctx, depth + 1, &operations),
                Operation::Or(operations) => execute_or(ctx, depth + 1, &operations),
                Operation::Query(query) => execute_query(ctx, depth + 1, &query),
            };

            results.push(result.docids);
            queries.extend(result.queries);
        }

        let results = results.iter().map(AsRef::as_ref).collect();
        let op = sdset::multi::Intersection::new(results);
        let docids = op.into_set_buf();

        println!("{:3$}--- AND fetched {} documents in {:.02?}",
            "", docids.len(), before.elapsed(), depth * 2);

        QueryResult { docids, queries }
    }

    fn execute_or<'a, 'c>(ctx: &'c Context, depth: usize, operations: &'a [Operation]) -> QueryResult<'a, 'c> {
        println!("{:1$}OR", "", depth * 2);

        let before = Instant::now();
        let mut queries = HashMap::new();
        let mut ids = Vec::new();

        for op in operations {
            let result = match op {
                Operation::And(operations) => execute_and(ctx, depth + 1, &operations),
                Operation::Or(operations) => execute_or(ctx, depth + 1, &operations),
                Operation::Query(query) => execute_query(ctx, depth + 1, &query),
            };

            ids.extend_from_slice(result.docids.as_ref());
            queries.extend(result.queries);
        }

        let docids = SetBuf::from_dirty(ids);

        println!("{:3$}--- OR fetched {} documents in {:.02?}",
            "", docids.len(), before.elapsed(), depth * 2);

        QueryResult { docids, queries }
    }

    fn execute_query<'a, 'c>(ctx: &'c Context, depth: usize, query: &'a Query) -> QueryResult<'a, 'c> {
        let before = Instant::now();
        let (docids, matches) = match query {
            Query::Tolerant(_, word) | Query::Exact(_, word) | Query::Prefix(_, word) => {
                if let Some(matches) = ctx.postings.get(word) {
                    let docids = matches.linear_group_by_key(|m| m.0).map(|g| g[0].0).collect();
                    (SetBuf::new(docids).unwrap(), Cow::Borrowed(matches.as_set()))
                } else {
                    (SetBuf::default(), Cow::default())
                }
            },
            Query::Phrase(_, words) => {
                if let [first, second] = words.as_slice() {
                    let default = SetBuf::default();
                    let first = ctx.postings.get(first).unwrap_or(&default);
                    let second = ctx.postings.get(second).unwrap_or(&default);

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

                    (SetBuf::new(docids).unwrap(), Cow::Owned(SetBuf::new(matches).unwrap()))
                } else {
                    println!("{:2$}{:?} skipped", "", words, depth * 2);
                    (SetBuf::default(), Cow::default())
                }
            },
        };

        println!("{:4$}{:?} fetched {:?} documents in {:.02?}",
            "", query, docids.len(), before.elapsed(), depth * 2);

        QueryResult {
            docids,
            queries: hashmap!{ query => matches },
        }
    }

    match tree {
        Operation::And(operations) => execute_and(ctx, 0, &operations),
        Operation::Or(operations) => execute_or(ctx, 0, &operations),
        Operation::Query(query) => execute_query(ctx, 0, &query),
    }
}

fn random_postings<R: Rng>(rng: &mut R, len: usize) -> SetBuf<(DocId, Position)> {
    let mut values = BTreeSet::new();
    while values.len() != len {
        values.insert(rng.gen());
    }

    let matches = values.iter().flat_map(|id| -> Vec<(DocId, Position)> {
        let mut matches = BTreeSet::new();
        let len = rng.gen_range(1, 10);
        while matches.len() != len {
            matches.insert(rng.gen());
        }
        matches.into_iter().map(|p| (*id, p)).collect()
    }).collect();

    SetBuf::new(matches).unwrap()
}

fn main() {
    let mut rng = StdRng::seed_from_u64(102);
    let rng = &mut rng;

    let context = Context {
        synonyms: hashmap!{
            S("hello") => vec![
                vec![S("hi")],
                vec![S("good"), S("morning")],
            ],
            S("world") => vec![
                vec![S("earth")],
                vec![S("nature")]
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
    let query_tree = create_query_tree(&context, &query);

    println!("{:?}", query_tree);

    println!("---------------------------------\n");

    let QueryResult { docids, queries } = traverse_query_tree(&context, &query_tree);
    println!("found {} documents", docids.len());

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
