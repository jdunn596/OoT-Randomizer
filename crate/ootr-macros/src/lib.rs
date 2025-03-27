use {
    std::{
        fmt,
        iter,
        mem,
        str::FromStr,
    },
    indexmap::IndexMap,
    proc_macro::TokenStream,
    proc_macro2::Span,
    quote::{
        ToTokens,
        quote,
    },
    syn::{
        *,
        spanned::Spanned as _,
    },
};

#[derive(PartialEq, Eq, Hash)]
enum NonEnglishLanguage {
    French,
    German,
}

impl FromStr for NonEnglishLanguage {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "French" => Ok(Self::French),
            "German" => Ok(Self::German),
            _ => Err(()),
        }
    }
}

impl fmt::Display for NonEnglishLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::French => write!(f, "French"),
            Self::German => write!(f, "German"),
        }
    }
}

enum Guard {
    If(Token![if], Expr),
    Let(ExprLet, Token![;]),
}

struct Translate {
    language: Expr,
    arms: IndexMap<NonEnglishLanguage, (Span, Vec<Guard>, Expr)>,
    english: Expr,
}

impl parse::Parse for Translate {
    fn parse(input: parse::ParseStream<'_>) -> Result<Self> {
        let language = input.parse()?;
        input.parse::<Token![;]>()?;
        let mut arms = IndexMap::default();
        let mut guards = Vec::default();
        loop {
            let lookahead = input.lookahead1();
            if lookahead.peek(Token![if]) {
                guards.push(Guard::If(input.parse()?, input.parse()?));
                input.parse::<Token![;]>()?;
            } else if lookahead.peek(Token![let]) {
                guards.push(Guard::Let(input.parse()?, input.parse()?));
            } else if lookahead.peek(Ident) {
                let ident = input.parse::<Ident>()?;
                input.parse::<Token![=>]>()?;
                if ident == "English" {
                    if !guards.is_empty() {
                        return Err(input.error("English translation cannot have match guards"))
                    }
                    let english = input.parse()?;
                    input.parse::<Token![;]>()?;
                    if !input.is_empty() {
                        return Err(input.error("translate macro must end after English match arm"))
                    }
                    return Ok(Self {
                        language,
                        arms,
                        english,
                    })
                } else {
                    let language = ident.to_string().parse().map_err(|()| input.error("unknown language"))?;
                    if arms.insert(language, (ident.span(), mem::take(&mut guards), input.parse()?)).is_some() {
                        return Err(input.error("duplicate language entry"))
                    }
                    input.parse::<Token![;]>()?;
                }
            } else {
                return Err(lookahead.error())
            }
        }
    }
}

/// A macro to translate text with [`if_chain`](https://docs.rs/if_chain)-style guards and automatic fallback to English.
#[proc_macro]
pub fn translate(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as Translate);
    let ident = Ident::new("__language", input.language.span());
    let mut fallback = input.english;
    for (language, (span, guards, mut expr)) in input.arms.into_iter().rev() {
        for guard in guards.into_iter().rev() {
            expr = match guard {
                Guard::If(if_token, cond) => Expr::If(ExprIf {
                    attrs: Vec::default(),
                    if_token,
                    cond: Box::new(cond),
                    then_branch: Block {
                        brace_token: token::Brace::default(),
                        stmts: vec![
                            Stmt::Expr(expr, None),
                        ],
                    },
                    else_branch: Some((Token![else](Span::call_site()), Box::new(Expr::Block(ExprBlock {
                        attrs: Vec::default(),
                        label: None,
                        block: Block {
                            brace_token: token::Brace::default(),
                            stmts: vec![
                                Stmt::Expr(fallback.clone(), None),
                            ],
                        },
                    })))),
                }),
                Guard::Let(ExprLet { attrs, let_token, pat, eq_token, expr }, semi_token) => Expr::Block(ExprBlock {
                    attrs: Vec::default(),
                    label: None,
                    block: Block {
                        brace_token: token::Brace::default(),
                        stmts: vec![
                            Stmt::Local(Local {
                                attrs,
                                let_token,
                                pat: *pat,
                                init: Some(LocalInit {
                                    eq_token,
                                    expr,
                                    diverge: None,
                                }),
                                semi_token,
                            }),
                        ],
                    },
                }),
            };
        }
        fallback = Expr::If(ExprIf {
            attrs: Vec::default(),
            if_token: Token![if](Span::call_site()),
            cond: Box::new(Expr::Binary(ExprBinary {
                attrs: Vec::default(),
                left: Box::new(Expr::Path(ExprPath {
                    attrs: Vec::default(),
                    qself: None,
                    path: Path {
                        leading_colon: None,
                        segments: iter::once(PathSegment::from(ident.clone())).collect(),
                    },
                })),
                op: BinOp::Eq(Token![==](Span::call_site())),
                right: Box::new(Expr::Block(ExprBlock {
                    attrs: Vec::default(),
                    label: None,
                    block: Block {
                        brace_token: token::Brace::default(),
                        stmts: vec![
                            Stmt::Expr(Expr::Path(ExprPath {
                                attrs: vec![
                                    Attribute {
                                        pound_token: Token![#](Span::call_site()),
                                        style: AttrStyle::Outer,
                                        bracket_token: token::Bracket::default(),
                                        meta: Meta::List(MetaList {
                                            path: Path {
                                                leading_colon: None,
                                                segments: iter::once(PathSegment::from(Ident::new("allow", Span::call_site()))).collect(),
                                            },
                                            delimiter: MacroDelimiter::Paren(token::Paren::default()),
                                            tokens: quote!(unused_qualifications),
                                        }),
                                    },
                                ],
                                qself: None,
                                path: Path {
                                    leading_colon: None,
                                    segments: [
                                        Token![crate](span).into(),
                                        Ident::new("lang", span),
                                        Ident::new("Language", span),
                                        Ident::new(&language.to_string(), span),
                                    ].into_iter().map(PathSegment::from).collect(),
                                },
                            }), None),
                        ],
                    },
                })),
            })),
            then_branch: Block {
                brace_token: token::Brace::default(),
                stmts: vec![
                    Stmt::Expr(expr, None),
                ],
            },
            else_branch: Some((Token![else](Span::call_site()), Box::new(Expr::Block(ExprBlock {
                attrs: Vec::default(),
                label: None,
                block: Block {
                    brace_token: token::Brace::default(),
                    stmts: vec![
                        Stmt::Expr(fallback, None),
                    ],
                },
            })))),
        });
    }
    TokenStream::from(Block {
        brace_token: token::Brace::default(),
        stmts: vec![
            Stmt::Local(Local {
                attrs: Vec::default(),
                let_token: Token![let](Span::call_site()),
                pat: Pat::Ident(PatIdent {
                    attrs: Vec::default(),
                    by_ref: None,
                    mutability: None,
                    ident,
                    subpat: None,
                }),
                init: Some(LocalInit {
                    eq_token: Token![=](Span::call_site()),
                    expr: Box::new(input.language),
                    diverge: None,
                }),
                semi_token: Token![;](Span::call_site()),
            }),
            Stmt::Expr(fallback, None),
        ],
    }.into_token_stream())
}
