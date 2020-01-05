use std::collections::{HashMap, BTreeSet, BTreeMap};
use std::iter::FromIterator;
use std::time::Instant;
use std::{cmp, fmt, iter, mem};

use big_s::S;
use maplit::hashmap;
use rand::{Rng, SeedableRng, rngs::StdRng};
use slice_group_by::StrGroupBy;
use itertools::{merge_join_by, EitherOrBoth};

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

#[derive(Debug)]
enum Query {
    Tolerant(String),
    Exact(String),
    Prefix(String),
    Phrase(Vec<String>),
}

impl Query {
    fn tolerant(s: &str) -> Query {
        Query::Tolerant(s.to_string())
    }

    fn prefix(s: &str) -> Query {
        Query::Prefix(s.to_string())
    }

    fn phrase2((left, right): (&str, &str)) -> Query {
        Query::Phrase(vec![left.to_owned(), right.to_owned()])
    }
}

type DocId = u16;
type Position = u8;

#[derive(Debug, Default)]
struct Context {
    synonyms: HashMap<String, Vec<Vec<String>>>,
    postings: HashMap<String, Vec<(DocId, Vec<Position>)>>,
}

fn split_best_frequency<'a>(ctx: &Context, word: &'a str) -> Option<(&'a str, &'a str)> {
    let chars = word.char_indices().skip(1);
    let mut best = None;

    for (i, _) in chars {
        let (left, right) = word.split_at(i);

        let left_freq = ctx.postings.get(left).map(Vec::len).unwrap_or(0);
        let right_freq = ctx.postings.get(right).map(Vec::len).unwrap_or(0);

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

fn ngram_slice<T>(ngram: usize, slice: &[T]) -> impl Iterator<Item=&[T]> {
    (0..slice.len()).flat_map(move |i| {
        (1..=ngram).into_iter().filter_map(move |n| slice.get(i..i + n))
    })
}

fn group_by<I, F>(iter: I, f: F) -> impl Iterator<Item=Vec<I::Item>>
where I: IntoIterator,
      F: Fn(&I::Item, &I::Item) -> bool,
{
    let mut iter = iter.into_iter();
    let mut prev = None;
    core::iter::from_fn(move || {
        let mut out = Vec::new();
        loop {
            match (prev.take().or_else(|| iter.next()), iter.next()) {
                (Some(a), Some(b)) if f(&a, &b) => {
                    out.push(a);
                    prev = Some(b);
                },
                (Some(a), Some(b)) => {
                    out.push(a);
                    prev = Some(b);
                    return Some(out);
                },
                (Some(a), None) => {
                    out.push(a);
                    return Some(out);
                },
                (None, _) => return None,
            }
        }
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

    let words = query.linear_group_by_key(char::is_whitespace);
    let words = is_last(words).filter(|(_, s)| !s.contains(char::is_whitespace)).enumerate();
    let words: Vec<_> = words.collect();

    let mut ands = Vec::new();
    for words in group_by(ngram_slice(MAX_NGRAM, &words), |a, b| a[0].0 == b[0].0) {

        let mut ops = Vec::new();
        for words in words {

            match words {
                [(_, (is_last, word))] => {
                    let phrase = split_best_frequency(ctx, word).map(Query::phrase2).map(Operation::Query);
                    let synonyms = synonyms(ctx, word).into_iter().map(|alts| {
                        let iter = alts.into_iter().map(Query::Exact).map(Operation::Query);
                        create_operation(iter, Operation::And)
                    });

                    let original = if *is_last {
                        Query::prefix(word)
                    } else {
                        Query::tolerant(word)
                    };

                    let mut alternatives: Vec<_> = synonyms.chain(phrase).collect();

                    if !alternatives.is_empty() {
                        ops.push(Operation::Query(original));
                        ops.append(&mut alternatives);
                    } else {
                        ops.push(Operation::Query(original));
                    }
                },
                words => {
                    let concat = words.iter().map(|(_, (_, s))| *s).collect();
                    ops.push(Operation::Query(Query::Exact(concat)));
                }
            }
        }

        let ops = create_operation(ops, Operation::Or);
        ands.push(ops)
    }

    create_operation(ands, Operation::And)
}

fn traverse_query_tree(ctx: &Context, tree: &Operation) -> HashMap<DocId, Vec<Position>> {

    fn execute_and(ctx: &Context, depth: usize, operations: &[Operation]) -> HashMap<DocId, Vec<Position>> {
        println!("{:1$}AND", "", depth * 2);

        let before = Instant::now();
        let mut ids = None;

        for op in operations {
            let result = match op {
                Operation::And(operations) => execute_and(ctx, depth + 1, &operations),
                Operation::Or(operations) => execute_or(ctx, depth + 1, &operations),
                Operation::Query(query) => execute_query(ctx, depth + 1, &query),
            };

            let before = Instant::now();
            match ids {
                Some(ref mut ids) => {
                    let mut old = mem::replace(ids, HashMap::new());
                    for (id, mut positions) in result {
                        if let Some(ref mut bpos) = old.remove(&id) {
                            positions.append(bpos);
                            ids.insert(id, positions);
                        }
                    }
                },
                None => ids = Some(result),
            }
            println!("AND loop took {:.02?}", before.elapsed());
        }

        let ids = ids.unwrap_or_default();

        println!("{:3$}--- AND fetched {} documents in {:.02?}",
            "", ids.len(), before.elapsed(), depth * 2);

        ids
    }

    fn execute_or(ctx: &Context, depth: usize, operations: &[Operation]) -> HashMap<DocId, Vec<Position>> {
        println!("{:1$}OR", "", depth * 2);

        let before = Instant::now();
        let mut ids = HashMap::new();

        for op in operations {
            let result = match op {
                Operation::And(operations) => execute_and(ctx, depth + 1, &operations),
                Operation::Or(operations) => execute_or(ctx, depth + 1, &operations),
                Operation::Query(query) => execute_query(ctx, depth + 1, &query),
            };

            let before = Instant::now();
            for (id, ref mut positions) in result {
                ids.entry(id)
                    .and_modify(|p: &mut Vec<_>| p.append(positions))
                    .or_insert_with(Vec::new);
            }
            println!("OR loop took {:.02?}", before.elapsed());
        }

        println!("{:3$}--- OR fetched {} documents in {:.02?}",
            "", ids.len(), before.elapsed(), depth * 2);

        ids
    }

    fn execute_query(ctx: &Context, depth: usize, query: &Query) -> HashMap<DocId, Vec<Position>> {
        match query {
            Query::Tolerant(word) | Query::Exact(word) | Query::Prefix(word) => {
                let before = Instant::now();

                if let Some(pl) = ctx.postings.get(word) {
                    println!("{:4$}{:?} fetched {:?} documents in {:.02?}",
                        "", word, pl.len(), before.elapsed(), depth * 2);
                    pl.into_iter().cloned().collect()
                } else {
                    println!("{:3$}{:?} fetched nothing in {:.02?}",
                        "", word, before.elapsed(), depth * 2);
                    HashMap::new()
                }
            },
            Query::Phrase(words) => {
                let before = Instant::now();

                let first = execute_query(ctx, depth + 1, &Query::Exact(words[0].to_owned()));
                let first = BTreeMap::from_iter(first);

                let second = execute_query(ctx, depth + 1, &Query::Exact(words[1].to_owned()));
                let second = BTreeMap::from_iter(second);

                let mut result = HashMap::new();
                for eob in merge_join_by(first, second, |a, b| a.0.cmp(&b.0)) {

                    if let EitherOrBoth::Both((id, left), (_, right)) = eob {

                        let mut positions = Vec::new();
                        for eob in merge_join_by(left, right, |a, b| (a + 1).cmp(&b)) {

                            if let EitherOrBoth::Both(a, b) = eob {
                                positions.push(a);
                                positions.push(b);
                            }
                        }

                        if !positions.is_empty() {
                            result.insert(id, positions);
                        }
                    }
                }

                println!("{:4$}{:?} fetched {:?} documents in {:.02?}",
                    "", words, result.len(), before.elapsed(), depth * 2);

                result
            },
        }
    }

    let mut result = match tree {
        Operation::And(operations) => execute_and(ctx, 0, &operations),
        Operation::Or(operations) => execute_or(ctx, 0, &operations),
        Operation::Query(query) => execute_query(ctx, 0, &query),
    };

    result.iter_mut().for_each(|(_, positions)| {
        positions.sort_unstable();
        positions.dedup();
    });

    result
}

fn random_docs<R: Rng>(rng: &mut R, len: usize) -> Vec<(DocId, Vec<Position>)> {
    let mut values = BTreeSet::new();
    while values.len() != len {
        values.insert(rng.gen());
    }
    values.into_iter().map(|id| {
        let len = rng.gen_range(1, 12);
        let positions = iter::repeat_with(|| rng.gen()).take(len).collect();
        (id, positions)
    }).collect()
}

fn main() {
    let mut rng = StdRng::seed_from_u64(42);
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
            S("hello")      => random_docs(rng,   1500),
            S("helloworld") => random_docs(rng,    100),
            S("hi")         => random_docs(rng,   4000),
            S("hell")       => random_docs(rng,   2500),
            S("o")          => random_docs(rng,    400),
            S("worl")       => random_docs(rng,   1400),
            S("world")      => random_docs(rng, 15_000),
            S("earth")      => random_docs(rng,   8000),
            S("2020")       => random_docs(rng,    100),
            S("2019")       => random_docs(rng,    500),
            S("is")         => random_docs(rng, 50_000),
            S("this")       => random_docs(rng, 50_000),
            S("good")       => random_docs(rng,   1250),
            S("morning")    => random_docs(rng,    125),
        },
    };

    let query = std::env::args().nth(1).unwrap_or(S("hello world"));
    let query_tree = create_query_tree(&context, &query);

    println!("{:?}", query_tree);

    println!("---------------------------------\n");

    let docids = traverse_query_tree(&context, &query_tree);

    println!("found {} documents", docids.len());
}
