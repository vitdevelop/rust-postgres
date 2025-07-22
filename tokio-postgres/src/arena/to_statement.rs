use crate::arena::statement::Statement;
use crate::arena::to_statement::private::{Sealed, ToStatementType};

mod private {
    use bumpalo::Bump;
    use crate::{Client, Error};
    use crate::arena::statement::Statement;

    pub trait Sealed {}

    pub enum ToStatementType<'a, 'b> {
        Statement(&'a Statement<'b>),
        Query(&'a str),
    }

    impl<'b> ToStatementType<'_, 'b> {
        pub async fn into_statement_in(self, client: &Client, arena: &'b Bump) -> Result<Statement<'b>, Error> {
            match self {
                ToStatementType::Statement(s) => Ok(s.clone()),
                ToStatementType::Query(s) => client.prepare_in(s, arena).await,
            }
        }
    }
}

/// A trait abstracting over prepared and unprepared statements.
///
/// Many methods are generic over this bound, so that they support both a raw query string as well as a statement which
/// was prepared previously.
///
/// This trait is "sealed" and cannot be implemented by anything outside this crate.
pub trait ToStatement: Sealed {
    #[doc(hidden)]
    fn __convert(&self) -> ToStatementType<'_, '_>;
}

impl ToStatement for Statement<'_> {
    fn __convert(&self) -> ToStatementType<'_, '_> {
        ToStatementType::Statement(self)
    }
}

impl Sealed for Statement<'_> {}

impl ToStatement for str {
    fn __convert(&self) -> ToStatementType<'_, '_> {
        ToStatementType::Query(self)
    }
}

impl Sealed for str {}

impl ToStatement for String {
    fn __convert(&self) -> ToStatementType<'_, '_> {
        ToStatementType::Query(self)
    }
}

impl Sealed for String {}
