use std::collections::HashMap;
use std::{cmp, fmt};

use maplit::hashmap;
use big_s::S;

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

    fn phrase<I, S>(iter: I) -> Operator
    where I: IntoIterator<Item=S>,
          S: std::fmt::Display,
    {
        Operator::Phrase(iter.into_iter().map(|s| s.to_string()).collect())
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

type Occurence = usize;

#[derive(Debug, Default)]
struct Context {
    synonyms: HashMap<String, Vec<String>>,
    words: HashMap<String, Occurence>,
}

fn split_best_frequency<'a>(ctx: &Context, word: &'a str) -> Option<(&'a str, &'a str)> {
    let chars = word.char_indices().skip(1);
    let mut best = None;

    for (i, _) in chars {
        let (left, right) = word.split_at(i);

        let left_freq = ctx.words.get(left).copied().unwrap_or(0);
        let right_freq = ctx.words.get(right).copied().unwrap_or(0);

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

fn create_query_tree(ctx: &Context, words: &[&str]) -> Operator {
    let mut ands = Vec::new();

    let count = words.len();
    for (i, word) in words.into_iter().enumerate() {

        let pq = split_best_frequency(ctx, word).map(|(l, r)| Operator::phrase(&[l, r]));
        let synonyms = synonyms(ctx, word);

        let mut alternatives: Vec<_> = synonyms.into_iter().map(Operator::Exact).chain(pq).collect();

        if !alternatives.is_empty() {

            let simple = if count - 1 == i { // is last?
                Operator::prefix(word)
            } else {
                Operator::exact(word)
            };

            alternatives.insert(0, simple);
            ands.push(Operator::Or(alternatives));
        } else {
            let simple = if count - 1 == i { // is last?
                Operator::prefix(word)
            } else {
                Operator::exact(word)
            };

            ands.push(simple);
        }
    }

    Operator::And(ands)
}

fn main() {
    let context = Context {
        synonyms: hashmap!{
            S("hello") => vec![S("hi")],
            S("world") => vec![S("earth"), S("nature")],
        },
        words: hashmap!{
            S("hell") => 25,
            S("o") => 4,
            S("worl") => 14,
        },
    };

    let words = &["hello", "world"];
    let query_tree = create_query_tree(&context, words);

    println!("{:?}", words);
    println!("{:?}", query_tree);
}
