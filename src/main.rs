use std::collections::{HashMap, HashSet, BTreeSet};
use std::{cmp, fmt};

use big_s::S;
use maplit::hashmap;
use slice_group_by::StrGroupBy;
use rand::{Rng, SeedableRng, rngs::StdRng};

enum Operator {
    And(Vec<Operator>),
    Or(Vec<Operator>),
    Phrase(Vec<String>),
    Prefix(String),
    Exact(String),
}

impl Operator {
    fn prefix(s: &str) -> Operator {
        Operator::Prefix(s.to_string())
    }

    fn exact(s: &str) -> Operator {
        Operator::Exact(s.to_string())
    }

    fn phrase2((left, right): (&str, &str)) -> Operator {
        Operator::Phrase(vec![left.to_owned(), right.to_owned()])
    }
}

impl fmt::Debug for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn pprint_tree(f: &mut fmt::Formatter<'_>, op: &Operator, depth: usize) -> fmt::Result {
            match op {
                Operator::And(children) => {
                    writeln!(f, "{:1$}AND", "", depth * 2)?;
                    children.iter().try_for_each(|c| pprint_tree(f, c, depth + 1))
                },
                Operator::Or(children) => {
                    writeln!(f, "{:1$}OR", "", depth * 2)?;
                    children.iter().try_for_each(|c| pprint_tree(f, c, depth + 1))
                },
                Operator::Phrase(phrase) => writeln!(f, "{:2$}PHRASE( {:?} )", "", phrase, depth * 2),
                Operator::Prefix(text) => writeln!(f, "{:2$}PREFIX( {:?} )", "", text, depth * 2),
                Operator::Exact(text) => writeln!(f, "{:2$}EXACT(  {:?} )", "", text, depth * 2),
            }
        }

        pprint_tree(f, self, 0)
    }
}

type DocId = u16;

#[derive(Debug, Default)]
struct Context {
    synonyms: HashMap<String, Vec<String>>,
    postings: HashMap<String, Vec<DocId>>,
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

fn synonyms(ctx: &Context, word: &str) -> Vec<String> {
    ctx.synonyms.get(word).cloned().unwrap_or_default()
}

fn is_last<I: IntoIterator>(iter: I) -> impl Iterator<Item=(bool, I::Item)> {
    let mut iter = iter.into_iter().peekable();
    core::iter::from_fn(move || {
        iter.next().map(|item| (iter.peek().is_none(), item))
    })
}

fn is_first<I: IntoIterator>(iter: I) -> impl Iterator<Item=(bool, I::Item)> {
    let mut iter = iter.into_iter();
    let mut is_first = true;
    core::iter::from_fn(move || {
        iter.next().map(|item| (core::mem::take(&mut is_first), item))
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

const MAX_NGRAM: usize = 3;

fn create_query_tree(ctx: &Context, query: &str) -> Operator {
    let query = query.to_lowercase();

    let words = query.linear_group_by_key(char::is_whitespace);
    let words: Vec<_> = is_last(words).filter(|(_, s)| !s.contains(char::is_whitespace)).collect();

    let mut ands = Vec::new();
    for words in group_by(ngram_slice(MAX_NGRAM, &words), |a, b| a[0].1 == b[0].1) {

        let mut ops = Vec::new();
        for words in words {

            match words {
                [(is_last, word)] => {
                    let phrase = split_best_frequency(ctx, word).map(Operator::phrase2);
                    let synonyms = synonyms(ctx, word).into_iter().map(Operator::Exact);

                    let original = if *is_last {
                        Operator::prefix(word)
                    } else {
                        Operator::exact(word)
                    };

                    let mut alternatives: Vec<_> = synonyms.chain(phrase).collect();

                    if !alternatives.is_empty() {
                        ops.push(original);
                        ops.append(&mut alternatives);
                    } else {
                        ops.push(original);
                    }
                },
                words => {
                    let concat = words.iter().map(|(_, s)| *s).collect();
                    ops.push(Operator::Exact(concat));
                }
            }
        }

        ands.push(Operator::Or(ops));
    }

    Operator::And(ands)
}

fn random_docs<R: Rng>(rng: &mut R, len: usize) -> Vec<DocId> {
    let mut values = BTreeSet::new();
    while values.len() != len {
        values.insert(rng.gen());
    }
    values.into_iter().collect()
}

fn main() {
    let mut rng = StdRng::seed_from_u64(42);
    let rng = &mut rng;

    let context = Context {
        synonyms: hashmap!{
            S("hello") => vec![S("hi")],
            S("world") => vec![S("earth"), S("nature")],
        },
        postings: hashmap!{
            S("hello")      => random_docs(rng, 1500),
            S("helloworld") => random_docs(rng, 100),
            S("hi")         => random_docs(rng, 4000),
            S("hell")       => random_docs(rng, 2500),
            S("o")          => random_docs(rng, 400),
            S("worl")       => random_docs(rng, 1400),
            S("world")      => random_docs(rng, 15000),
            S("earth")      => random_docs(rng, 8000),
            S("2020")       => random_docs(rng, 100),
            S("2019")       => random_docs(rng, 500),
            S("is")         => random_docs(rng, 50000),
            S("this")       => random_docs(rng, 50000),
        },
    };

    let query = std::env::args().nth(1).unwrap_or(S("hello world"));
    let query_tree = create_query_tree(&context, &query);

    println!("{:?}", query_tree);

    println!("---------------------------------\n");

    match query_tree {
        Operator::And(ops) => {
            let mut and_ids = HashSet::new();

            for (is_first, op) in is_first(ops) {
                match op {
                    Operator::Or(ops) => {
                        let mut or_ids = HashSet::new();

                        for op in &ops {
                            match op {
                                Operator::Exact(word) => {
                                    let mut word_ids = HashSet::<DocId>::new();

                                    if let Some(ids) = context.postings.get(word) {
                                        word_ids.extend(ids);
                                    }

                                    println!("  {:?} retrieve {} documents", word, word_ids.len());
                                    or_ids.extend(word_ids);
                                },
                                Operator::Prefix(word) => {
                                    let mut word_ids = HashSet::<DocId>::new();

                                    if let Some(ids) = context.postings.get(word) {
                                        word_ids.extend(ids);
                                    }

                                    println!("  {:?}* retrieve {} documents", word, word_ids.len());
                                    or_ids.extend(word_ids);
                                },
                                op => println!("  ignored"),
                            }
                        }

                        println!("OP retrieve {} documents", or_ids.len());

                        if is_first {
                            and_ids = or_ids;
                        } else {
                            let old = std::mem::replace(&mut and_ids, HashSet::new());
                            and_ids.extend(old.intersection(&or_ids));
                        }
                        println!("AND as now {} documents", and_ids.len());
                    },
                    _ => unimplemented!(),
                }

            }

        },
        _ => unimplemented!(),
    }
}
