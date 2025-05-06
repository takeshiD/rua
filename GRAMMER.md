# BNF
[Section 9 of the Lua 5.4 ReferenceManual](https://www.lua.org/manual/5.4/manual.html#9)
```
chunk ::= block

block ::= {stat} [retstat]

stat ::=  ‘;’ | 
     varlist ‘=’ explist | 
     functioncall | 
     label | 
     break | 
     goto Name | 
     do block end | 
     while exp do block end | 
     repeat block until exp | 
     if exp then block {elseif exp then block} [else block] end | 
     for Name ‘=’ exp ‘,’ exp [‘,’ exp] do block end | 
     for namelist in explist do block end | 
     function funcname funcbody | 
     local function Name funcbody | 
     local attnamelist [‘=’ explist] 

attnamelist ::=  Name attrib {‘,’ Name attrib}

attrib ::= [‘<’ Name ‘>’]

retstat ::= return [explist] [‘;’]

label ::= ‘::’ Name ‘::’

funcname ::= Name {‘.’ Name} [‘:’ Name]

varlist ::= var {‘,’ var}

var ::=  Name | prefixexp ‘[’ exp ‘]’ | prefixexp ‘.’ Name 

namelist ::= Name {‘,’ Name}

explist ::= exp {‘,’ exp}

exp ::=  nil | false | true | Numeral | LiteralString | ‘...’ | functiondef | 
     prefixexp | tableconstructor | exp binop exp | unop exp 

prefixexp ::= var | functioncall | ‘(’ exp ‘)’

functioncall ::=  prefixexp args | prefixexp ‘:’ Name args 

args ::=  ‘(’ [explist] ‘)’ | tableconstructor | LiteralString 

functiondef ::= function funcbody

funcbody ::= ‘(’ [parlist] ‘)’ block end

parlist ::= namelist [‘,’ ‘...’] | ‘...’

tableconstructor ::= ‘{’ [fieldlist] ‘}’

fieldlist ::= field {fieldsep field} [fieldsep]

field ::= ‘[’ exp ‘]’ ‘=’ exp | Name ‘=’ exp | exp

fieldsep ::= ‘,’ | ‘;’

binop ::=  ‘+’ | ‘-’ | ‘*’ | ‘/’ | ‘//’ | ‘^’ | ‘%’ | 
     ‘&’ | ‘~’ | ‘|’ | ‘>>’ | ‘<<’ | ‘..’ | 
     ‘<’ | ‘<=’ | ‘>’ | ‘>=’ | ‘==’ | ‘~=’ | 
     and | or

unop ::= ‘-’ | not | ‘#’ | ‘~’
```


# References

- [Lua Get Started](https://lua.org/start.html)
- [Lua 5.1 Reference Manual](https://www.lua.org/manual/5.1/manual.html)
