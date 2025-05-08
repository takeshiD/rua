use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, digit1, multispace0, multispace1},
    combinator::{map, opt, recognize},
    error::ParseError,
    multi::{fold_many0, many0, separated_list0},
    number::complete::recognize_float,
    sequence::{delimited, pair, preceded, terminated, tuple},
    Finish, IResult, Parser,
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
enum Exp {
    Value(ExpKind),
    BinaryOp(BinaryOp, Box<Exp>, Box<Exp>),
    UnaryOp(UnaryOp, Box<Exp>),
}

fn space_delimited<'src, O, E>(
    f: impl Parser<&'src str, O, E>,
) -> impl FnMut(&'src str) -> IResult<&'src str, O, E>
where
    E: ParseError<&'src str>,
{
    delimited(multispace0, f, multispace0)
}

fn nil(i: &str) -> IResult<&str, ExpKind> {
    let (i, _) = space_delimited(tag("nil"))(i)?;
    Ok((i, ExpKind::NiL))
}

fn bool(i: &str) -> IResult<&str, ExpKind> {
    let (i, val) = space_delimited(alt((tag("true"), tag("false"))))(i)?;
    match val {
        "true" => Ok((i, ExpKind::True)),
        "false" => Ok((i, ExpKind::False)),
        _ => panic!("bool parsing failed"),
    }
}

fn float(i: &str) -> IResult<&str, ExpKind> {
    let (i, val) = space_delimited(alt((recognize_float, preceded(tag("-"), recognize_float))))(i)?;
    let val = val.parse::<LuaNumber>().unwrap();
    Ok((i, ExpKind::Float(val)))
}

fn integer(i: &str) -> IResult<&str, ExpKind> {
    space_delimited(alt((
        map(digit1, |s: &str| {
            ExpKind::Integer(s.parse::<LuaNumber>().unwrap())
        }),
        map(preceded(tag("-"), digit1), |s: &str| {
            ExpKind::Integer(-s.parse::<LuaNumber>().unwrap())
        }),
    )))(i)
}

fn expr_unary_op(i: &str) -> IResult<&str, Exp> {
    let (i, unop) = alt((tag("-"), tag("not "), tag("#")))(i)?;
    let (i, e) = expr(i)?;
    match unop {
        "-" => Ok((i, Exp::UnaryOp(UnaryOp::Minus, Box::new(e)))),
        "not " => Ok((i, Exp::UnaryOp(UnaryOp::Not, Box::new(e)))),
        "#" => Ok((i, Exp::UnaryOp(UnaryOp::Length, Box::new(e)))),
        _ => panic!("unary operator failed"),
    }
}

fn primary(i: &str) -> IResult<&str, Exp> {
    let (i, val) = alt((nil, bool, integer, float))(i)?;
    Ok((i, Exp::Value(val)))
}

fn term(i: &str) -> IResult<&str, Exp> {
    let (i, init) = primary(i)?;
    fold_many0(
        pair(space_delimited(alt((char('*'), char('/')))), primary),
        move || init.clone(),
        |acc, (op, val): (char, Exp)| match op {
            '*' => Exp::BinaryOp(BinaryOp::Mul, Box::new(acc), Box::new(val)),
            '/' => Exp::BinaryOp(BinaryOp::Div, Box::new(acc), Box::new(val)),
            _ => {
                panic!("Binary Operator must be '*' '/'.")
            }
        },
    )(i)
}

fn expr(i: &str) -> IResult<&str, Exp> {
    let (i, init) = term(i)?;
    fold_many0(
        pair(space_delimited(alt((char('+'), char('-')))), term),
        move || init.clone(),
        |acc, (op, val): (char, Exp)| match op {
            '+' => Exp::BinaryOp(BinaryOp::Add, Box::new(acc), Box::new(val)),
            '-' => Exp::BinaryOp(BinaryOp::Sub, Box::new(acc), Box::new(val)),
            _ => {
                panic!("Additive expression should have '+' or '-' operator")
            }
        },
    )(i)
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
                    Box::new(
                        Exp::BinaryOp(
                            BinaryOp::Sub, 
                            Box::new(Exp::Value(ExpKind::Integer(1.0))), 
                            Box::new(Exp::Value(ExpKind::Integer(2.0)))
                        )
                    ),
                    Box::new(
                        Exp::Value(ExpKind::Integer(3.0))
                    ),
                )
            ))
        );
    }
}
