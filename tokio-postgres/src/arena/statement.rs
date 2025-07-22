use crate::client::InnerClient;
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::types::Type;
use postgres_protocol::message::frontend;
use std::sync::{Arc, Weak};

struct StatementInner<'a> {
    client: Weak<InnerClient>,
    name: bumpalo::collections::String<'a>,
    params: bumpalo::collections::Vec<'a, Type>,
    columns: bumpalo::collections::Vec<'a, Column<'a>>,
}

impl Drop for StatementInner<'_> {
    fn drop(&mut self) {
        if self.name.is_empty() {
            // Unnamed statements don't need to be closed
            return;
        }
        if let Some(client) = self.client.upgrade() {
            let buf = client.with_buf(|buf| {
                frontend::close(b'S', &self.name, buf).unwrap();
                frontend::sync(buf);
                buf.split().freeze()
            });
            let _ = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)));
        }
    }
}

/// A prepared statement.
///
/// Prepared statements can only be used with the connection that created them.
#[derive(Clone)]
pub struct Statement<'a>(Arc<StatementInner<'a>>);

impl<'a> Statement<'a> {
    pub(crate) fn new(
        inner: &Arc<InnerClient>,
        name: bumpalo::collections::String<'a>,
        params: bumpalo::collections::Vec<'a, Type>,
        columns: bumpalo::collections::Vec<'a, Column<'_>>,
    ) -> Statement<'a> {
        Statement(Arc::new(StatementInner {
            client: Arc::downgrade(inner),
            name,
            params,
            columns,
        }))
    }

    pub(crate) fn unnamed_in(
        params: bumpalo::collections::Vec<'a, Type>,
        columns: bumpalo::collections::Vec<'a, Column<'_>>,
    ) -> Statement<'a> {
        Statement(Arc::new(StatementInner {
            client: Weak::new(),
            name: bumpalo::collections::String::new_in(columns.bump()),
            params,
            columns,
        }))
    }

    pub(crate) fn name(&self) -> &str {
        &self.0.name
    }

    /// Returns the expected types of the statement's parameters.
    pub fn params(&self) -> &[Type] {
        &self.0.params
    }

    /// Returns information about the columns returned when the statement is queried.
    pub fn columns(&self) -> &[Column<'_>] {
        &self.0.columns
    }
}

impl std::fmt::Debug for Statement<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Statement")
            .field("name", &self.0.name)
            .field("params", &self.0.params)
            .field("columns", &self.0.columns)
            .finish_non_exhaustive()
    }
}

/// Information about a column of a query.
#[derive(Debug)]
pub struct Column<'a> {
    pub(crate) name: bumpalo::collections::String<'a>,
    pub(crate) table_oid: Option<u32>,
    pub(crate) column_id: Option<i16>,
    pub(crate) r#type: Type,
}

impl Column<'_> {
    /// Returns the name of the column.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the OID of the underlying database table.
    pub fn table_oid(&self) -> Option<u32> {
        self.table_oid
    }

    /// Return the column ID within the underlying database table.
    pub fn column_id(&self) -> Option<i16> {
        self.column_id
    }

    /// Returns the type of the column.
    pub fn type_(&self) -> &Type {
        &self.r#type
    }
}
