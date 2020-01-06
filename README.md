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
      Tolerant(0, "hello")
      Exact(0, "hi")
      AND
        Exact(0, "good")
        Exact(0, "morning")
      Phrase(0, ["hell", "o"])
    OR
      Tolerant(1, "world")
      Exact(1, "earth")
      Exact(1, "nature")
    Prefix(2, "2020")
  AND
    Exact(0, "helloworld")
    Prefix(2, "2020")
  AND
    OR
      Tolerant(0, "hello")
      Exact(0, "hi")
      AND
        Exact(0, "good")
        Exact(0, "morning")
      Phrase(0, ["hell", "o"])
    Exact(1, "world2020")
  Exact(0, "helloworld2020")

---------------------------------

OR
  AND
    OR
      Tolerant(0, "hello") fetched 1500 documents in 16.40µs
      Exact(0, "hi") fetched 4000 documents in 39.66µs
      AND
        Exact(0, "good") fetched 1250 documents in 13.06µs
        Exact(0, "morning") fetched 125 documents in 2.00µs
      --- AND fetched 2 documents in 24.28µs
      matches [(1617, 69), (1617, 70)]
      Phrase(0, ["hell", "o"]) fetched 1 documents in 49.83µs
    --- OR fetched 5403 documents in 279.10µs
    OR
      Tolerant(1, "world") fetched 15000 documents in 153.56µs
      Exact(1, "earth") fetched 8000 documents in 76.03µs
      Exact(1, "nature") fetched 0 documents in 101.00ns
    --- OR fetched 21145 documents in 683.94µs
    Prefix(2, "2020") fetched 100 documents in 2.10µs
  --- AND fetched 4 documents in 987.65µs
  AND
    Exact(0, "helloworld") fetched 100 documents in 1.91µs
    Prefix(2, "2020") fetched 100 documents in 1.72µs
  --- AND fetched 0 documents in 9.59µs
  AND
    OR
      Tolerant(0, "hello") fetched 1500 documents in 15.60µs
      Exact(0, "hi") fetched 4000 documents in 39.89µs
      AND
        Exact(0, "good") fetched 1250 documents in 13.26µs
        Exact(0, "morning") fetched 125 documents in 1.99µs
      --- AND fetched 2 documents in 23.52µs
      matches [(1617, 69), (1617, 70)]
      Phrase(0, ["hell", "o"]) fetched 1 documents in 49.65µs
    --- OR fetched 5403 documents in 235.38µs
    Exact(1, "world2020") fetched 0 documents in 77.00ns
  --- AND fetched 0 documents in 240.87µs
  Exact(0, "helloworld2020") fetched 0 documents in 56.00ns
--- OR fetched 4 documents in 1.25ms
found 4 documents
Exact(1, "earth") gives 6 matches
Exact(0, "hi") gives 7 matches
Tolerant(1, "world") gives 14 matches
Prefix(2, "2020") gives 25 matches
Tolerant(0, "hello") gives 2 matches
matches cleaned in 11.09µs
```
