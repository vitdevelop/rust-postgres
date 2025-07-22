use crate::arena::statement::{Column, Statement};
use crate::client::InnerClient;
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::prepare::{get_type, NEXT_ID};
use crate::types::Type;
use crate::Error;
use bumpalo::Bump;
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use log::debug;
use postgres_protocol::message::backend::Message;
use postgres_protocol::message::frontend;
use std::fmt::Write;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub async fn prepare_in<'a>(
    client: &Arc<InnerClient>,
    query: &str,
    types: &[Type],
    arena: &'a Bump,
) -> Result<Statement<'a>, Error> {
    let mut name = bumpalo::collections::string::String::new_in(arena);
    match std::write!(name, "s{}", NEXT_ID.fetch_add(1, Ordering::SeqCst)) {
        Ok(_) => {}
        Err(err) => return Err(Error::config(Box::new(err))),
    }

    let buf = encode(client, &name, query, types)?;
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    match responses.next().await? {
        Message::ParseComplete => {}
        _ => return Err(Error::unexpected_message()),
    }

    let parameter_description = match responses.next().await? {
        Message::ParameterDescription(body) => body,
        _ => return Err(Error::unexpected_message()),
    };

    let row_description = match responses.next().await? {
        Message::RowDescription(body) => Some(body),
        Message::NoData => None,
        _ => return Err(Error::unexpected_message()),
    };

    let mut parameters = bumpalo::collections::Vec::new_in(arena);
    let mut it = parameter_description.parameters();
    while let Some(oid) = it.next().map_err(Error::parse)? {
        let type_ = get_type(client, oid).await?;
        parameters.push(type_);
    }

    let mut columns = bumpalo::collections::Vec::new_in(arena);
    if let Some(row_description) = row_description {
        let mut it = row_description.fields();
        while let Some(field) = it.next().map_err(Error::parse)? {
            let type_ = get_type(client, field.type_oid()).await?;
            let column = Column {
                name: bumpalo::collections::String::from_str_in(field.name(), arena),
                table_oid: Some(field.table_oid()).filter(|n| *n != 0),
                column_id: Some(field.column_id()).filter(|n| *n != 0),
                r#type: type_,
            };
            columns.push(column);
        }
    }

    Ok(Statement::new(client, name, parameters, columns))
}

fn encode(client: &InnerClient, name: &str, query: &str, types: &[Type]) -> Result<Bytes, Error> {
    if types.is_empty() {
        debug!("preparing query {}: {}", name, query);
    } else {
        debug!("preparing query {} with types {:?}: {}", name, types, query);
    }

    client.with_buf(|buf| {
        frontend::parse(name, query, types.iter().map(Type::oid), buf).map_err(Error::encode)?;
        frontend::describe(b'S', name, buf).map_err(Error::encode)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })
}
