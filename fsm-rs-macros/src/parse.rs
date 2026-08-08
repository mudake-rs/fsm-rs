use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, parenthesized, Ident, LitBool, Token, Type};

use crate::model::*;

impl Parse for MachineDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut context = None;
        let mut serde = false;
        let mut states = None;
        let mut events = None;
        let mut transitions = None;
        let mut unhandled = None;
        let mut on_transition = None;

        while !input.is_empty() {
            let kw: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            match kw.to_string().as_str() {
                "name" => set_once(&mut name, input.parse()?, &kw)?,
                "context" => set_once(&mut context, input.parse::<Type>()?, &kw)?,
                "serde" => serde = input.parse::<LitBool>()?.value,
                "states" => {
                    let content;
                    braced!(content in input);
                    set_once(&mut states, parse_states(&content)?, &kw)?;
                }
                "events" => {
                    let content;
                    braced!(content in input);
                    set_once(&mut events, parse_events(&content)?, &kw)?;
                }
                "transitions" => {
                    let content;
                    braced!(content in input);
                    set_once(&mut transitions, parse_rows(&content)?, &kw)?;
                }
                "unhandled" => {
                    let value = parse_unhandled(input)?;
                    set_once(&mut unhandled, value, &kw)?;
                }
                "on_transition" => {
                    set_once(&mut on_transition, parse_callable(input)?, &kw)?;
                }
                other => {
                    return Err(syn::Error::new(
                        kw.span(),
                        format!(
                            "unknown key `{other}`; expected one of: \
                             name, context, serde, states, events, transitions, unhandled, on_transition"
                        ),
                    ));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(MachineDef {
            name: name.ok_or_else(|| input.error("missing required key `name`"))?,
            context: context.ok_or_else(|| input.error("missing required key `context`"))?,
            serde,
            states: states.ok_or_else(|| input.error("missing required key `states`"))?,
            events: events.ok_or_else(|| input.error("missing required key `events`"))?,
            transitions: transitions
                .ok_or_else(|| input.error("missing required key `transitions`"))?,
            unhandled,
            on_transition,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, kw: &Ident) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(kw.span(), format!("duplicate key `{kw}`")));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_callable(input: ParseStream) -> syn::Result<Callable> {
    let is_async = input.peek(Token![async]);
    if is_async {
        input.parse::<Token![async]>()?;
    }
    let name: Ident = input.parse()?;
    Ok(Callable { is_async, name })
}

fn parse_unhandled(input: ParseStream) -> syn::Result<Unhandled> {
    let is_async = input.peek(Token![async]);
    if is_async {
        input.parse::<Token![async]>()?;
    }
    let name: Ident = input.parse()?;
    if !is_async && name == "ignore" {
        Ok(Unhandled::Ignore)
    } else {
        Ok(Unhandled::Method(Callable { is_async, name }))
    }
}

fn parse_states(input: ParseStream) -> syn::Result<Vec<StateDef>> {
    let mut states = Vec::new();
    while !input.is_empty() {
        let initial = input.peek(Token![*]);
        if initial {
            input.parse::<Token![*]>()?;
        }
        let name: Ident = input.parse()?;
        let mut entry = None;
        let mut exit = None;
        if input.peek(syn::token::Paren) {
            let content;
            parenthesized!(content in input);
            while !content.is_empty() {
                let kw: Ident = content.parse()?;
                content.parse::<Token![:]>()?;
                match kw.to_string().as_str() {
                    "entry" => set_once(&mut entry, parse_callable(&content)?, &kw)?,
                    "exit" => set_once(&mut exit, parse_callable(&content)?, &kw)?,
                    other => {
                        return Err(syn::Error::new(
                            kw.span(),
                            format!("unknown state option `{other}`; expected `entry` or `exit`"),
                        ));
                    }
                }
                if content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                } else {
                    break;
                }
            }
        }
        states.push(StateDef {
            name,
            initial,
            entry,
            exit,
        });
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(states)
}

fn parse_events(input: ParseStream) -> syn::Result<Vec<Ident>> {
    let mut events = Vec::new();
    while !input.is_empty() {
        events.push(input.parse::<Ident>()?);
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(events)
}

fn parse_rows(input: ParseStream) -> syn::Result<Vec<Row>> {
    let mut rows = Vec::new();
    while !input.is_empty() {
        let span = input.span();

        let sources = if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            SourcePattern::Any
        } else {
            let mut list = Vec::new();
            loop {
                let initial_marker = input.peek(Token![*]);
                if initial_marker {
                    input.parse::<Token![*]>()?;
                }
                list.push(StateRef {
                    name: input.parse()?,
                    initial_marker,
                });
                if input.peek(Token![|]) {
                    input.parse::<Token![|]>()?;
                } else {
                    break;
                }
            }
            SourcePattern::States(list)
        };

        input.parse::<Token![+]>()?;

        let events = if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            EventPattern::Any
        } else {
            let mut list = Vec::new();
            loop {
                list.push(input.parse::<Ident>()?);
                if input.peek(Token![|]) {
                    input.parse::<Token![|]>()?;
                } else {
                    break;
                }
            }
            EventPattern::Events(list)
        };

        let guard = if input.peek(syn::token::Bracket) {
            let content;
            bracketed!(content in input);
            Some(parse_callable(&content)?)
        } else {
            None
        };

        let action = if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            Some(parse_callable(input)?)
        } else {
            None
        };

        input.parse::<Token![=>]>()?;

        let target = if input.peek(Token![_]) {
            input.parse::<Token![_]>()?;
            Target::Stay
        } else {
            Target::State(input.parse()?)
        };

        rows.push(Row {
            sources,
            events,
            guard,
            action,
            target,
            span,
        });

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else if !input.is_empty() {
            return Err(input.error("expected `,` between transitions"));
        }
    }
    Ok(rows)
}
