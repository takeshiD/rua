use full_moon::{Error, ast::Ast};

pub fn parse(source: &str) -> Result<Ast, Vec<Error>> {
    full_moon::parse(source)
}
