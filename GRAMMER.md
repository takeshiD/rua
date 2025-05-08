# BNF
- [Lua Get Started](https://lua.org/start.html)
- [Lua 5.1 Reference Manual](https://www.lua.org/manual/5.1/manual.html)

`[ symbol ]` is optional(none or once).

`{ symbol }` is repetiton(none or more).

```
chunk ::= {stat [`;´]} [laststat [`;´]]

block ::= chunk

stat ::=  varlist `=´ explist | 
     functioncall | 
     do block end | 
     while exp do block end | 
     repeat block until exp | 
     if exp then block {elseif exp then block} [else block] end | 
     for Name `=´ exp `,´ exp [`,´ exp] do block end | 
     for namelist in explist do block end | 
     function funcname funcbody | 
     local function Name funcbody | 
     local namelist [`=´ explist] 

laststat ::= return [explist] | break

funcname ::= Name {`.´ Name} [`:´ Name]

varlist ::= var {`,´ var}

var ::=  Name | prefixexp `[´ exp `]´ | prefixexp `.´ Name 

namelist ::= Name {`,´ Name}

explist ::= {exp `,´} exp

exp ::=  nil | false | true | Number | String | `...´ | function | 
     prefixexp | tableconstructor | exp binop exp | unop exp 

exp = term { "+" term | "-" term }*
term = primary { "*" primary | "/" primary }
primary = nil | false | true | Number | String | `...´ | function | tableconstructor | prefixexp | unop exp

prefixexp ::= var | functioncall | `(´ exp `)´

functioncall ::=  prefixexp args | prefixexp `:´ Name args 

args ::=  `(´ [explist] `)´ | tableconstructor | String 

function ::= function funcbody

funcbody ::= `(´ [parlist] `)´ block end

parlist ::= namelist [`,´ `...´] | `...´

tableconstructor ::= `{´ [fieldlist] `}´

fieldlist ::= field {fieldsep field} [fieldsep]

field ::= `[´ exp `]´ `=´ exp | Name `=´ exp | exp

fieldsep ::= `,´ | `;´

binop ::= `+´ | `-´ | `*´ | `/´ | `^´ | `%´ | `..´ | 
     `<´ | `<=´ | `>´ | `>=´ | `==´ | `~=´ | 
     and | or

unop ::= `-´ | not | `#´```

