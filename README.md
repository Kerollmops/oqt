# oqt
Stands for optimal query tree. A POC of the new MeiliSearch internal query tree.

## Example output

```bash
cargo run --release -- 'hello world 2020'
```

```
OR
  AND
    OR
      Tolerant { id: 0, word: "hello" }
      Exact { id: 102, word: "hi" }
      AND
        Exact { id: 103, word: "good" }
        Exact { id: 104, word: "morning" }
      Phrase { id: 100, words: ["hell", "o"] }
    OR
      AND
        OR
          Tolerant { id: 1, word: "world" }
          Exact { id: 200, word: "earth" }
          Exact { id: 201, word: "nature" }
        PrefixTolerant { id: 2, word: "2020" }
      PrefixExact { id: 20000, word: "world2020" }
  AND
    OR
      AND
        Exact { id: 10000, word: "bonjour" }
        Exact { id: 10001, word: "monde" }
      Exact { id: 10002, word: "helloworld" }
    PrefixTolerant { id: 2, word: "2020" }
  PrefixExact { id: 1000000, word: "helloworld2020" }

{
    0: 0..2,
    1: 2..3,
    2: 3..4,
    100: 0..1,
    101: 1..2,
    102: 0..2,
    103: 0..1,
    104: 1..2,
    200: 2..3,
    201: 2..3,
    10000: 0..1,
    10001: 1..3,
    10002: 0..3,
    20000: 2..4,
    1000000: 0..4,
}
---------------------------------

OR
  AND
    OR
      Tolerant { id: 0, word: "hello" } fetched 1500 documents in 262.00ns
      Exact { id: 102, word: "hi" } fetched 4000 documents in 76.00ns
      AND
        Exact { id: 103, word: "good" } fetched 1250 documents in 89.00ns
        Exact { id: 104, word: "morning" } fetched 125 documents in 74.00ns
      --- AND fetched 2 documents in 10.89µs
      matches [(1617, 69), (1617, 70)]
      Phrase { id: 100, words: ["hell", "o"] } fetched 1 documents in 52.01µs
    --- OR fetched 5403 documents in 199.72µs
    OR
      AND
        OR
          Tolerant { id: 1, word: "world" } fetched 15000 documents in 145.00ns
          Exact { id: 200, word: "earth" } fetched 8000 documents in 191.00ns
          Exact { id: 201, word: "nature" } fetched 0 documents in 80.00ns
        --- OR fetched 21145 documents in 472.30µs
        PrefixTolerant { id: 2, word: "2020" } fetched 100 documents in 145.00ns
      --- AND fetched 37 documents in 487.37µs
      PrefixExact { id: 20000, word: "world2020" } fetched 0 documents in 65.00ns
    --- OR fetched 37 documents in 492.77µs
  --- AND fetched 4 documents in 707.61µs
  AND
    OR
      AND
        Exact { id: 10000, word: "bonjour" } fetched 0 documents in 91.00ns
        Exact { id: 10001, word: "monde" } fetched 0 documents in 54.00ns
      --- AND fetched 0 documents in 4.30µs
      Exact { id: 10002, word: "helloworld" } fetched 100 documents in 89.00ns
    --- OR fetched 100 documents in 8.64µs
  --- AND fetched 0 documents in 13.43µs
  PrefixExact { id: 1000000, word: "helloworld2020" } fetched 0 documents in 82.00ns
--- OR fetched 4 documents in 730.42µs
found 4 documents
number of postings 14
Exact { id: 200, word: "earth" } gives 6 matches
Tolerant { id: 0, word: "hello" } gives 2 matches
Tolerant { id: 1, word: "world" } gives 14 matches
PrefixTolerant { id: 2, word: "2020" } gives 25 matches
Exact { id: 102, word: "hi" } gives 7 matches
matches cleaned in 17.71µs
```
