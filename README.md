# oqt
Stands for optimal query tree. A POC of the new MeiliSearch internal query tree.

## Example output

```bash
cargo run --release -- 'hello world this is 2020'
```

```
AND
  OR
    Tolerant(0, "hello")
    Exact(0, "hi")
    AND
      Exact(0, "good")
      Exact(0, "morning")
    Phrase(0, ["hell", "o"])
    Exact(0, "helloworld")
    Exact(0, "helloworldthis")
  OR
    Tolerant(1, "world")
    Exact(1, "earth")
    Exact(1, "nature")
    Exact(1, "worldthis")
    Exact(1, "worldthisis")
  OR
    Tolerant(2, "this")
    Exact(2, "thisis")
    Exact(2, "thisis2020")
  OR
    Tolerant(3, "is")
    Exact(3, "is2020")
  Prefix(4, "2020")

---------------------------------

AND
  OR
    Tolerant(0, "hello") fetched 1500 documents in 24.95µs
    Exact(0, "hi") fetched 4000 documents in 55.21µs
    AND
      Exact(0, "good") fetched 1250 documents in 18.08µs
      Exact(0, "morning") fetched 125 documents in 2.73µs
    --- AND fetched 2 documents in 33.39µs
    matches [(1617, 69), (1617, 70)]
    Phrase(0, ["hell", "o"]) fetched 1 documents in 66.70µs
    Exact(0, "helloworld") fetched 100 documents in 2.23µs
    Exact(0, "helloworldthis") fetched 0 documents in 94.00ns
  --- OR fetched 5499 documents in 368.74µs
  OR
    Tolerant(1, "world") fetched 15000 documents in 218.51µs
    Exact(1, "earth") fetched 8000 documents in 108.07µs
    Exact(1, "nature") fetched 0 documents in 130.00ns
    Exact(1, "worldthis") fetched 0 documents in 99.00ns
    Exact(1, "worldthisis") fetched 0 documents in 103.00ns
  --- OR fetched 21145 documents in 990.83µs
  OR
    Tolerant(2, "this") fetched 50000 documents in 701.31µs
    Exact(2, "thisis") fetched 0 documents in 105.00ns
    Exact(2, "thisis2020") fetched 0 documents in 85.00ns
  --- OR fetched 50000 documents in 820.38µs
  OR
    Tolerant(3, "is") fetched 50000 documents in 704.93µs
    Exact(3, "is2020") fetched 0 documents in 87.00ns
  --- OR fetched 50000 documents in 776.44µs
  Prefix(4, "2020") fetched 100 documents in 2.94µs
--- AND fetched 3 documents in 3.02ms
found 3 documents
Exact(0, "hi") gives 7 matches
Prefix(4, "2020") gives 18 matches
Tolerant(3, "is") gives 10 matches
Exact(1, "earth") gives 6 matches
Tolerant(2, "this") gives 16 matches
Tolerant(1, "world") gives 13 matches
matches cleaned in 24.58µs
```
