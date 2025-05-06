use nom::{
    Finish, IResult, Parser,
    branch::alt,
    bytes::complete::tag,
    character::complete::{alpha1, alphanumeric1, char, multispace0, multispace1},
    combinator::{opt, recognize},
    error::ParseError,
    multi::{fold_many0, many0, separated_list0},
    number::complete::recognize_float,
    sequence::{delimited, pair, preceded, terminated},
};

type LuaNumber = f64;
#[derive(Debug, PartialEq)]
enum ExpKind {
  NiL,
  True,
  False,
  Float(LuaNumber),
  Integer(LuaNumber),
}

#[derive(Debug, PartialEq)]
enum BinaryOp {
    Add,    // '+'
    Sub,    // '-'
    Mul,    // '*'
    Div,    // '/'
}

#[derive(Debug, PartialEq)]
enum UnaryOp {
    Minus,  // '-'
    Not,    // 'not'
    Length, // '#'
    BNot,   // '~'
}

#[derive(Debug, PartialEq)]
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
    let (i, val) = space_delimited(recognize_float)(i)?;
    let val = val.parse::<LuaNumber>().unwrap();
    Ok((i, ExpKind::Float(val)))
}

// fn integer(i: &str) -> IResult<&str, Exp> {
//     let (i, val) = space_delimited(recognize_float)(i)?;
//     Ok((i, ExpKind::Float(val)))
// }

fn expr_value(i: &str) -> IResult<&str, Exp> {
    let (i, val) = alt((nil, bool, float))(i)?;
    Ok((i, Exp::Value(val)))
}

fn expr(i: &str) -> IResult<&str, Exp> {
    expr_value(i)
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
    }
}
