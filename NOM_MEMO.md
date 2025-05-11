# version 8.0.0
生成されるコード量削減などのために以下の破壊的変更

- クロージャーベースの実装からトレイトベースに変更(このため`combinator(arg)(input)` は `combinator(arg).parse(input)`と書くことになる)
- 



# 便利なパーサー
## `space_delimited`
[Rustで作るプログラミング言語](https://www.amazon.co.jp/Rust%E3%81%A7%E4%BD%9C%E3%82%8B%E3%83%97%E3%83%AD%E3%82%B0%E3%83%A9%E3%83%9F%E3%83%B3%E3%82%B0%E8%A8%80%E8%AA%9E-%E2%80%94%E2%80%94-%E3%82%B3%E3%83%B3%E3%83%91%E3%82%A4%E3%83%A9%EF%BC%8F%E3%82%A4%E3%83%B3%E3%82%BF%E3%83%97%E3%83%AA%E3%82%BF%E3%81%AE%E5%9F%BA%E7%A4%8E%E3%81%8B%E3%82%89%E3%83%97%E3%83%AD%E3%82%B0%E3%83%A9%E3%83%9F%E3%83%B3%E3%82%B0%E8%A8%80%E8%AA%9E%E3%81%AE%E6%96%B0%E6%BD%AE%E6%B5%81%E3%81%BE%E3%81%A7-%E4%BD%90%E4%B9%85%E7%94%B0-%E6%98%8C%E5%8D%9A/dp/4297141922)の中で出てくる便利なパーサー

書籍の中ではv7のnomを利用しているのでv8になると若干構成が変わります。

v7での場合(書籍内で定義する構造体Spanが入力なってますがなんとなくわかるかと思います)
```rust
fn space_delimited<'src, O, E>(
  f: impl Parser<Span<'src>, O, E>,
) -> impl FnMut(Span<'src>) -> IResult<Span<'src>, O, E>
where
  E: ParseError<Span<'src>>,
{
  delimited(multispace0, f, multispace0)
}
```


v8では以下のようになります。

```rust
fn space_delimited<I, O, E: ParseError<I>, F>(
    f: F,
) -> impl Parser<I, Output=O, Error=E>
where
    I: Input,
    <I as Input>::Item: AsChar,
    F: Parser<I, Output=O, Error=E>
{
    delimited(multispace0, f, multispace0)
}
```


一番の違いは返り値が`impl FnMut -> IResult`から`impl Parser`に変わったことです。

これはv7まではパーサーをクロージャーで構成していましたが、v8からはtraitで実装し直したためです。

破壊的な変更なのでご注意ください。

