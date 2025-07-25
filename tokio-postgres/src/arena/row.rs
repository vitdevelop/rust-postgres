//! Rows.

use crate::arena::row::sealed::{AsName, Sealed};
use crate::arena::statement::{Column, Statement};
use crate::simple_query::SimpleColumn;
use crate::types::{FromSql, WrongType};
use crate::Error;
use bumpalo::Bump;
use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::DataRowBody;
use std::fmt;
use std::ops::Range;
use std::str;

mod sealed {
    pub trait Sealed {}

    pub trait AsName {
        fn as_name(&self) -> &str;
    }
}

impl AsName for Column<'_> {
    fn as_name(&self) -> &str {
        self.name()
    }
}

impl AsName for String {
    fn as_name(&self) -> &str {
        self
    }
}

/// A trait implemented by types that can index into columns of a row.
///
/// This cannot be implemented outside of this crate.
pub trait RowIndex: Sealed {
    #[doc(hidden)]
    fn __idx<T>(&self, columns: &[T]) -> Option<usize>
    where
        T: AsName;
}

impl Sealed for usize {}

impl RowIndex for usize {
    #[inline]
    fn __idx<T>(&self, columns: &[T]) -> Option<usize>
    where
        T: AsName,
    {
        if *self >= columns.len() {
            None
        } else {
            Some(*self)
        }
    }
}

impl Sealed for str {}

impl RowIndex for str {
    #[inline]
    fn __idx<T>(&self, columns: &[T]) -> Option<usize>
    where
        T: AsName,
    {
        if let Some(idx) = columns.iter().position(|d| d.as_name() == self) {
            return Some(idx);
        };

        // FIXME ASCII-only case insensitivity isn't really the right thing to
        // do. Postgres itself uses a dubious wrapper around tolower and JDBC
        // uses the US locale.
        columns
            .iter()
            .position(|d| d.as_name().eq_ignore_ascii_case(self))
    }
}

impl<T> Sealed for &T where T: ?Sized + Sealed {}

impl<T> RowIndex for &T
where
    T: ?Sized + RowIndex,
{
    #[inline]
    fn __idx<U>(&self, columns: &[U]) -> Option<usize>
    where
        U: AsName,
    {
        T::__idx(*self, columns)
    }
}

/// A row of data returned from the database by a query.
#[derive(Clone)]
pub struct Row<'a> {
    statement: Statement<'a>,
    body: DataRowBody,
    ranges: bumpalo::collections::Vec<'a, Option<Range<usize>>>,
}

impl fmt::Debug for Row<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row")
            .field("columns", &self.columns())
            .finish()
    }
}

impl<'a> Row<'a> {
    pub(crate) fn new(
        statement: Statement<'a>,
        body: DataRowBody,
        arena: &'a Bump,
    ) -> Result<Row<'a>, Error> {
        let ranges = body
            .ranges()
            .try_fold(
                bumpalo::collections::Vec::new_in(arena),
                |mut vec, range| {
                    vec.push(range);
                    Ok(vec)
                },
            )
            .map_err(Error::parse)?;

        Ok(Row {
            statement,
            body,
            ranges,
        })
    }

    /// Returns information about the columns of data in the row.
    pub fn columns(&self) -> &[Column<'_>] {
        self.statement.columns()
    }

    /// Determines if the row contains no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of values in the row.
    pub fn len(&self) -> usize {
        self.columns().len()
    }

    /// Deserializes a value from the row.
    ///
    /// The value can be specified either by its numeric index in the row, or by its column name.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds or if the value cannot be converted to the specified type.
    #[track_caller]
    pub fn get<'b, I, T>(&'b self, idx: I) -> T
    where
        I: RowIndex + fmt::Display,
        T: FromSql<'b>,
    {
        match self.get_inner(&idx) {
            Ok(ok) => ok,
            Err(err) => panic!("error retrieving column {}: {}", idx, err),
        }
    }

    /// Like `Row::get`, but returns a `Result` rather than panicking.
    pub fn try_get<'b, I, T>(&'b self, idx: I) -> Result<T, Error>
    where
        I: RowIndex + fmt::Display,
        T: FromSql<'b>,
    {
        self.get_inner(&idx)
    }

    fn get_inner<'b, I, T>(&'b self, idx: &I) -> Result<T, Error>
    where
        I: RowIndex + fmt::Display,
        T: FromSql<'b>,
    {
        let idx = match idx.__idx(self.columns()) {
            Some(idx) => idx,
            None => return Err(Error::column(idx.to_string())),
        };

        let ty = self.columns()[idx].type_();
        if !T::accepts(ty) {
            return Err(Error::from_sql(
                Box::new(WrongType::new::<T>(ty.clone())),
                idx,
            ));
        }

        FromSql::from_sql_nullable(ty, self.col_buffer(idx)).map_err(|e| Error::from_sql(e, idx))
    }

    /// Get the raw bytes for the column at the given index.
    fn col_buffer(&self, idx: usize) -> Option<&[u8]> {
        let range = self.ranges[idx].to_owned()?;
        Some(&self.body.buffer()[range])
    }

    /// Clean the body bytes
    pub unsafe fn clean_body(&mut self) {
        self.body.clean();
        self.ranges.clear();
    }
}

impl AsName for SimpleColumn {
    fn as_name(&self) -> &str {
        self.name()
    }
}
