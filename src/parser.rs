use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, digit1, multispace0, multispace1},
    combinator::{map, opt, peek, recognize},
    error::{Error, ErrorKind, ParseError},
    multi::{fold_many0, many0, separated_list0},
    number::complete::recognize_float,
    sequence::{delimited, pair, preceded, terminated},
    AsChar, Finish, IResult, Input, Parser,
};

type LuaNumber = f64;

#[derive(Debug, PartialEq, Clone)]
enum ExpKind {
    NiL,
    True,
    False,
    Float(LuaNumber),
    Integer(LuaNumber),
}

#[derive(Debug, PartialEq, Clone)]
enum BinaryOp {
    Add, // '+'
    Sub, // '-'
    Mul, // '*'
    Div, // '/'
}

#[derive(Debug, PartialEq, Clone)]
enum UnaryOp {
    Minus,  // '-'
    Not,    // 'not'
    Length, // '#'
}

#[derive(Debug, PartialEq, Clone)]
enum Exp<'a> {
    Value(ExpKind),
    BinaryOp(BinaryOp, Box<Exp<'a>>, Box<Exp<'a>>),
    UnaryOp(UnaryOp, Box<Exp<'a>>),
    Identifier(&'a str),
}

/// remove spaces parser
/// # Examples
///
/// ```
/// assert_eq!(parens_delimited(digit1).parse(" 12   "), Ok("", "12"))
/// ```
fn spaces_delimited<I, O, E: ParseError<I>, F>(f: F) -> impl Parser<I, Output = O, Error = E>
where
    I: Input,
    <I as Input>::Item: AsChar,
    F: Parser<I, Output = O, Error = E>,
{
    delimited(multispace0, f, multispace0)
}

/// encloses parens(`(` and `)`) parser
/// # Examples
///
/// ```
/// assert_eq!(parens_delimited(digit1).parse("( 12)"), Ok("", "12"))
/// assert_eq!(parens_delimited(digit1).parse(" 12)"), Err())
/// ```
fn parens_delimited<I, O, E: ParseError<I>, F>(f: F) -> impl Parser<I, Output = O, Error = E>
where
    I: Input,
    <I as Input>::Item: AsChar,
    F: Parser<I, Output = O, Error = E>,
{
    delimited(spaces_delimited(char('(')), f, spaces_delimited(char(')')))
}

/// encloses square brackets(`[` and `]`) parser
/// # Examples
///
/// ```
/// assert_eq!(parens_delimited(digit1).parse("[ 12]"), Ok("", "12"))
/// assert_eq!(parens_delimited(digit1).parse(" 12]"), Err())
/// ```
fn square_brackets_delimited<I, O, E: ParseError<I>, F>(
    f: F,
) -> impl Parser<I, Output = O, Error = E>
where
    I: Input,
    <I as Input>::Item: AsChar,
    F: Parser<I, Output = O, Error = E>,
{
    delimited(spaces_delimited(char('[')), f, spaces_delimited(char(']')))
}

/// encloses curly brackets(`{` and `}`) parser
/// # Examples
///
/// ```
/// assert_eq!(parens_delimited(digit1).parse("{ 12}"), Ok("", "12"))
/// assert_eq!(parens_delimited(digit1).parse(" 12}"), Err())
/// ```
fn curly_brackets_delimited<I, O, E: ParseError<I>, F>(
    f: F,
) -> impl Parser<I, Output = O, Error = E>
where
    I: Input,
    <I as Input>::Item: AsChar,
    F: Parser<I, Output = O, Error = E>,
{
    delimited(spaces_delimited(char('{')), f, spaces_delimited(char('}')))
}

fn nil(i: &str) -> IResult<&str, ExpKind> {
    let (i, _) = spaces_delimited(tag("nil")).parse(i)?;
    Ok((i, ExpKind::NiL))
}

fn bool(i: &str) -> IResult<&str, ExpKind> {
    let (i, val) = spaces_delimited(alt((tag("true"), tag("false")))).parse(i)?;
    match val {
        "true" => Ok((i, ExpKind::True)),
        "false" => Ok((i, ExpKind::False)),
        _ => panic!("bool parsing failed"),
    }
}

fn float(i: &str) -> IResult<&str, ExpKind> {
    let (i, val) =
        spaces_delimited(alt((recognize_float, preceded(tag("-"), recognize_float)))).parse(i)?;
    let val = val.parse::<LuaNumber>().unwrap();
    Ok((i, ExpKind::Float(val)))
}

fn integer(i: &str) -> IResult<&str, ExpKind> {
    spaces_delimited(alt((
        map(digit1, |s: &str| {
            ExpKind::Integer(s.parse::<LuaNumber>().unwrap())
        }),
        map(preceded(tag("-"), digit1), |s: &str| {
            ExpKind::Integer(-s.parse::<LuaNumber>().unwrap())
        }),
    )))
    .parse(i)
}

fn value(i: &str) -> IResult<&str, Exp> {
    let (i, val) = alt((nil, bool, integer, float)).parse(i)?;
    Ok((i, Exp::Value(val)))
}

fn is_reserved(i: &str) -> bool {
    let reserved_words = [
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
        "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ];
    reserved_words.contains(&i)
}

/// Lua Name(identifier) parser
fn identifier(i: &str) -> IResult<&str, Exp> {
    let (i, id) = recognize(pair(
        alt((alpha1, tag("_"))),
        many0(alt((alphanumeric1, tag("_")))),
    ))
    .parse(i)?;
    if is_reserved(id) {
        return Err(nom::Err::Error(Error::new(id, ErrorKind::Tag)));
    }
    Ok((i, Exp::Identifier(id)))
}

/// Lua variable parser
/// var ::= Name | prefixexp `[´ exp `]´ | prefixexp `.´ Name
/// # Todos
/// - implement: prefixexp `[` exp `]`
/// - implement: prefixexp `.` Name
fn variable(i: &str) -> IResult<&str, Exp> {
    alt((identifier,)).parse(i)
}

fn prefix_expr(i: &str) -> IResult<&str, Exp> {
    alt((variable, parens_delimited(expr))).parse(i)
}

fn unary_expr(i: &str) -> IResult<&str, Exp> {
    let (i, unop) = alt((tag("-"), tag("not "), tag("#"))).parse(i)?;
    let (i, e) = expr(i)?;
    match unop {
        "-" => Ok((i, Exp::UnaryOp(UnaryOp::Minus, Box::new(e)))),
        "not " => Ok((i, Exp::UnaryOp(UnaryOp::Not, Box::new(e)))),
        "#" => Ok((i, Exp::UnaryOp(UnaryOp::Length, Box::new(e)))),
        _ => panic!("unary operator failed"),
    }
}

fn primary(i: &str) -> IResult<&str, Exp> {
    let (i, e) = alt((value, unary_expr, prefix_expr)).parse(i)?;
    Ok((i, e))
}

fn term(i: &str) -> IResult<&str, Exp> {
    let (i, init) = primary(i)?;
    fold_many0(
        pair(spaces_delimited(alt((char('*'), char('/')))), primary),
        move || init.clone(),
        |acc, (op, val): (char, Exp)| match op {
            '*' => Exp::BinaryOp(BinaryOp::Mul, Box::new(acc), Box::new(val)),
            '/' => Exp::BinaryOp(BinaryOp::Div, Box::new(acc), Box::new(val)),
            _ => {
                panic!("Binary Operator must be '*' '/'.")
            }
        },
    )
    .parse(i)
}

fn expr(i: &str) -> IResult<&str, Exp> {
    let (i, init) = term(i)?;
    fold_many0(
        pair(spaces_delimited(alt((char('+'), char('-')))), term),
        move || init.clone(),
        |acc, (op, val): (char, Exp)| match op {
            '+' => Exp::BinaryOp(BinaryOp::Add, Box::new(acc), Box::new(val)),
            '-' => Exp::BinaryOp(BinaryOp::Sub, Box::new(acc), Box::new(val)),
            _ => {
                panic!("Additive expression should have '+' or '-' operator")
            }
        },
    )
    .parse(i)
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_nil() {
        assert_eq!(nil("nil"), Ok(("", ExpKind::NiL)));
    }
    #[test]
    fn test_bool() {
        assert_eq!(bool("true"), Ok(("", ExpKind::True)));
        assert_eq!(bool("false"), Ok(("", ExpKind::False)));
    }
    #[test]
    fn test_float() {
        assert_eq!(float("3.0"), Ok(("", ExpKind::Float(3.0))));
        assert_eq!(float("3.1416"), Ok(("", ExpKind::Float(3.1416))));
        assert_eq!(float("314.16e-2"), Ok(("", ExpKind::Float(3.1416))));
        assert_eq!(float("0.31416E1"), Ok(("", ExpKind::Float(3.1416))));
        assert_eq!(float("34e1"), Ok(("", ExpKind::Float(340f64))));
        assert_eq!(float("0.0"), Ok(("", ExpKind::Float(0.0))));
        assert_eq!(float("-3.0"), Ok(("", ExpKind::Float(-3.0))));
    }
    #[test]
    fn test_integer() {
        assert_eq!(integer("3"), Ok(("", ExpKind::Integer(3.0))));
        assert_eq!(integer("-3"), Ok(("", ExpKind::Integer(-3.0))));
        assert_eq!(integer("0"), Ok(("", ExpKind::Integer(0.0))));
    }
    #[test]
    fn test_identifier() {
        assert_eq!(
            identifier("hello_name"),
            Ok(("", Exp::Identifier("hello_name")))
        );
        assert_eq!(identifier("_hello"), Ok(("", Exp::Identifier("_hello"))));
        assert_eq!(
            identifier("and"),
            Err(nom::Err::Error(Error::new("and", ErrorKind::Tag)))
        );
        assert_eq!(
            identifier("or"),
            Err(nom::Err::Error(Error::new("or", ErrorKind::Tag)))
        );
    }
    #[test]
    fn test_primary() {
        assert_eq!(primary("nil"), Ok(("", Exp::Value(ExpKind::NiL))));
        assert_eq!(primary("(nil)"), Ok(("", Exp::Value(ExpKind::NiL))));
        assert_eq!(primary(" ( nil ) "), Ok(("", Exp::Value(ExpKind::NiL))));
        assert_eq!(primary(" ( nil ) "), Ok(("", Exp::Value(ExpKind::NiL))));
        assert_eq!(
            primary("not true"),
            Ok((
                "",
                Exp::UnaryOp(UnaryOp::Not, Box::new(Exp::Value(ExpKind::True)))
            ))
        );
        assert_eq!(
            primary("not not true"),
            Ok((
                "",
                Exp::UnaryOp(
                    UnaryOp::Not,
                    Box::new(Exp::UnaryOp(
                        UnaryOp::Not,
                        Box::new(Exp::Value(ExpKind::True))
                    ))
                )
            ))
        );
    }
    #[test]
    fn test_expr() {
        assert_eq!(
            expr("1 + 1"),
            Ok((
                "",
                Exp::BinaryOp(
                    BinaryOp::Add,
                    Box::new(Exp::Value(ExpKind::Integer(1.0))),
                    Box::new(Exp::Value(ExpKind::Integer(1.0))),
                )
            ))
        );
        assert_eq!(
            expr("1-1"),
            Ok((
                "",
                Exp::BinaryOp(
                    BinaryOp::Sub,
                    Box::new(Exp::Value(ExpKind::Integer(1.0))),
                    Box::new(Exp::Value(ExpKind::Integer(1.0))),
                )
            ))
        );
        assert_eq!(
            expr("1 - 2 + 3"),
            Ok((
                "",
                Exp::BinaryOp(
                    BinaryOp::Add,
                    Box::new(Exp::BinaryOp(
                        BinaryOp::Sub,
                        Box::new(Exp::Value(ExpKind::Integer(1.0))),
                        Box::new(Exp::Value(ExpKind::Integer(2.0)))
                    )),
                    Box::new(Exp::Value(ExpKind::Integer(3.0))),
                )
            ))
        );
        assert_eq!(
            expr("(1 - 2) + 3"),
            Ok((
                "",
                Exp::BinaryOp(
                    BinaryOp::Add,
                    Box::new(Exp::BinaryOp(
                        BinaryOp::Sub,
                        Box::new(Exp::Value(ExpKind::Integer(1.0))),
                        Box::new(Exp::Value(ExpKind::Integer(2.0)))
                    )),
                    Box::new(Exp::Value(ExpKind::Integer(3.0))),
                )
            ))
        );
        assert_eq!(
            expr("(1 - 2) * 3"),
            Ok((
                "",
                Exp::BinaryOp(
                    BinaryOp::Mul,
                    Box::new(Exp::BinaryOp(
                        BinaryOp::Sub,
                        Box::new(Exp::Value(ExpKind::Integer(1.0))),
                        Box::new(Exp::Value(ExpKind::Integer(2.0)))
                    )),
                    Box::new(Exp::Value(ExpKind::Integer(3.0))),
                )
            ))
        );
        assert_eq!(
            expr("1 - 2 * 3"),
            Ok((
                "",
                Exp::BinaryOp(
                    BinaryOp::Sub,
                    Box::new(Exp::Value(ExpKind::Integer(1.0))),
                    Box::new(Exp::BinaryOp(
                        BinaryOp::Mul,
                        Box::new(Exp::Value(ExpKind::Integer(2.0))),
                        Box::new(Exp::Value(ExpKind::Integer(3.0)))
                    )),
                )
            ))
        );
    }
}
