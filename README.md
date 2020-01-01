# oqt
Stands for optimal query tree. A POC of the new MeiliSearch internal query tree.

## Example output

```
["hello", "world"]
AND
  OR
    EXACT(  "hello" )
    EXACT(  "hi" )
    PHRASE( ["hell", "o"] )
  OR
    PREFIX( "world" )
    EXACT(  "earth" )
    EXACT(  "nature" )
```
