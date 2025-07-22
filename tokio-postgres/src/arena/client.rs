//! Client

use crate::arena::prepare::prepare_in;
use crate::arena::query;
use crate::arena::query::RowStream;
use crate::arena::row::Row;
use crate::arena::statement::Statement;
use crate::arena::to_statement::ToStatement;
#[cfg(feature = "runtime")]
use crate::types::{ToSql, Type};
use crate::{slice_iter, Client, Error};
use bumpalo::Bump;
use futures_util::{pin_mut, TryStreamExt};
use postgres_types::BorrowToSql;

impl Client {
    /// Executes a statement, returning a vector of the resulting rows.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// The `statement` argument can either be a `Statement`, or a raw query string. If the same statement will be
    /// repeatedly executed (perhaps with different query parameters), consider preparing the statement up front
    /// with the `prepare` method.
    pub async fn query_in<'a, T>(
        &self,
        statement: &'a T,
        params: &[&(dyn ToSql + Sync)],
        arena: &'a Bump,
    ) -> Result<bumpalo::collections::Vec<'a, Row<'a>>, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.query_raw_in(statement, slice_iter(params), arena)
            .await?
            .try_fold(
                bumpalo::collections::Vec::new_in(arena),
                |mut vec, row| async {
                    vec.push(row);
                    Ok(vec)
                },
            )
            .await
    }

    /// Executes a statement which returns a single row, returning it.
    ///
    /// Returns an error if the query does not return exactly one row.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// The `statement` argument can either be a `Statement`, or a raw query string. If the same statement will be
    /// repeatedly executed (perhaps with different query parameters), consider preparing the statement up front
    /// with the `prepare` method.
    pub async fn query_one_in<'a, T>(
        &self,
        statement: &'a T,
        params: &[&(dyn ToSql + Sync)],
        arena: &'a Bump,
    ) -> Result<Row<'a>, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.query_opt_in(statement, params, arena)
            .await
            .and_then(|res| res.ok_or_else(Error::row_count))
    }

    /// Executes a statements which returns zero or one rows, returning it.
    ///
    /// Returns an error if the query returns more than one row.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// The `statement` argument can either be a `Statement`, or a raw query string. If the same statement will be
    /// repeatedly executed (perhaps with different query parameters), consider preparing the statement up front
    /// with the `prepare` method.
    pub async fn query_opt_in<'a, T>(
        &self,
        statement: &'a T,
        params: &[&(dyn ToSql + Sync)],
        arena: &'a Bump,
    ) -> Result<Option<Row<'a>>, Error>
    where
        T: ?Sized + ToStatement,
    {
        let stream = self
            .query_raw_in(statement, slice_iter(params), arena)
            .await?;
        pin_mut!(stream);

        let mut first = None;

        // Originally this was two calls to `try_next().await?`,
        // once for the first element, and second to error if more than one.
        //
        // However, this new form with only one .await in a loop generates
        // slightly smaller codegen/stack usage for the resulting future.
        while let Some(row) = stream.try_next().await? {
            if first.is_some() {
                return Err(Error::row_count());
            }

            first = Some(row);
        }

        Ok(first)
    }

    /// The maximally flexible version of [`query`].
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// The `statement` argument can either be a `Statement`, or a raw query string. If the same statement will be
    /// repeatedly executed (perhaps with different query parameters), consider preparing the statement up front
    /// with the `prepare` method.
    ///
    /// [`query`]: #method.query
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn async_main(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    /// use futures_util::{pin_mut, TryStreamExt};
    ///
    /// let params: Vec<String> = vec![
    ///     "first param".into(),
    ///     "second param".into(),
    /// ];
    /// let mut it = client.query_raw(
    ///     "SELECT foo FROM bar WHERE biz = $1 AND baz = $2",
    ///     params,
    /// ).await?;
    ///
    /// pin_mut!(it);
    /// while let Some(row) = it.try_next().await? {
    ///     let foo: i32 = row.get("foo");
    ///     println!("foo: {}", foo);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn query_raw_in<'a, T, P, I>(
        &self,
        statement: &'a T,
        params: I,
        arena: &'a Bump,
    ) -> Result<RowStream<'a>, Error>
    where
        T: ?Sized + ToStatement,
        P: BorrowToSql,
        I: IntoIterator<Item = P>,
        I::IntoIter: ExactSizeIterator,
    {
        let statement = statement.__convert().into_statement_in(self, arena).await?;
        query::query_in(&self.inner, statement, params, arena).await
    }

    /// Like `query`, but requires the types of query parameters to be explicitly specified.
    ///
    /// Compared to `query`, this method allows performing queries without three round trips (for
    /// prepare, execute, and close) by requiring the caller to specify parameter values along with
    /// their Postgres type. Thus, this is suitable in environments where prepared statements aren't
    /// supported (such as Cloudflare Workers with Hyperdrive).
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the
    /// parameter of the list provided, 1-indexed.
    pub async fn query_typed_in<'a>(
        &self,
        query: &str,
        params: &[(&(dyn ToSql + Sync), Type)],
        arena: &'a Bump,
    ) -> Result<bumpalo::collections::Vec<'a, Row<'a>>, Error> {
        self.query_typed_raw_in(query, params.iter().map(|(v, t)| (*v, t.clone())), arena)
            .await?
            .try_fold(
                bumpalo::collections::Vec::new_in(arena),
                |mut vec, row| async {
                    vec.push(row);
                    Ok(vec)
                },
            )
            .await
    }

    /// The maximally flexible version of [`query_typed`].
    ///
    /// Compared to `query`, this method allows performing queries without three round trips (for
    /// prepare, execute, and close) by requiring the caller to specify parameter values along with
    /// their Postgres type. Thus, this is suitable in environments where prepared statements aren't
    /// supported (such as Cloudflare Workers with Hyperdrive).
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the
    /// parameter of the list provided, 1-indexed.
    ///
    /// [`query_typed`]: #method.query_typed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn async_main(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    /// use futures_util::{pin_mut, TryStreamExt};
    /// use tokio_postgres::types::Type;
    ///
    /// let params: Vec<(String, Type)> = vec![
    ///     ("first param".into(), Type::TEXT),
    ///     ("second param".into(), Type::TEXT),
    /// ];
    /// let mut it = client.query_typed_raw(
    ///     "SELECT foo FROM bar WHERE biz = $1 AND baz = $2",
    ///     params,
    /// ).await?;
    ///
    /// pin_mut!(it);
    /// while let Some(row) = it.try_next().await? {
    ///     let foo: i32 = row.get("foo");
    ///     println!("foo: {}", foo);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn query_typed_raw_in<'a, P, I>(
        &self,
        query: &str,
        params: I,
        arena: &'a Bump,
    ) -> Result<RowStream<'a>, Error>
    where
        P: BorrowToSql,
        I: IntoIterator<Item = (P, Type)>,
    {
        query::query_typed_in(&self.inner, query, params, arena).await
    }

    /// Executes a statement, returning the number of rows modified.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// The `statement` argument can either be a `Statement`, or a raw query string. If the same statement will be
    /// repeatedly executed (perhaps with different query parameters), consider preparing the statement up front
    /// with the `prepare` method.
    ///
    /// If the statement does not modify any rows (e.g. `SELECT`), 0 is returned.
    pub async fn execute_in<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
        arena: &Bump,
    ) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.execute_raw_in(statement, slice_iter(params), arena)
            .await
    }

    /// The maximally flexible version of [`execute`].
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// The `statement` argument can either be a `Statement`, or a raw query string. If the same statement will be
    /// repeatedly executed (perhaps with different query parameters), consider preparing the statement up front
    /// with the `prepare` method.
    ///
    /// [`execute`]: #method.execute
    pub async fn execute_raw_in<T, P, I>(
        &self,
        statement: &T,
        params: I,
        arena: &Bump,
    ) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement,
        P: BorrowToSql,
        I: IntoIterator<Item = P>,
        I::IntoIter: ExactSizeIterator,
    {
        let statement = statement.__convert().into_statement_in(self, arena).await?;
        query::execute_in(self.inner(), statement, params, arena).await
    }

    /// Creates a new prepared statement.
    ///
    /// Prepared statements can be executed repeatedly, and may contain query parameters (indicated by `$1`, `$2`, etc),
    /// which are set when executed. Prepared statements can only be used with the connection that created them.
    pub async fn prepare_in<'a>(&self, query: &str, arena: &'a Bump) -> Result<Statement<'a>, Error> {
        self.prepare_typed_in(query, &[], arena).await
    }

    /// Like `prepare`, but allows the types of query parameters to be explicitly specified.
    ///
    /// The list of types may be smaller than the number of parameters - the types of the remaining parameters will be
    /// inferred. For example, `client.prepare_typed(query, &[])` is equivalent to `client.prepare(query)`.
    pub async fn prepare_typed_in<'a>(
        &self,
        query: &str,
        parameter_types: &[Type],
        arena: &'a Bump,
    ) -> Result<Statement<'a>, Error> {
        prepare_in(&self.inner, query, parameter_types, arena).await
    }
}
