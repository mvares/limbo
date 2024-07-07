use crate::util::normalize_ident;
use anyhow::Result;
use core::fmt;
use fallible_iterator::FallibleIterator;
use log::trace;
use sqlite3_parser::ast::TableOptions;
use sqlite3_parser::{
    ast::{Cmd, CreateTableBody, QualifiedName, ResultColumn, Stmt},
    lexer::sql::Parser,
};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Schema {
    pub tables: HashMap<String, Rc<BTreeTable>>,
}

impl Schema {
    pub fn new() -> Self {
        let mut tables: HashMap<String, Rc<BTreeTable>> = HashMap::new();
        tables.insert("sqlite_schema".to_string(), Rc::new(sqlite_schema_table()));
        Self { tables }
    }

    pub fn add_table(&mut self, table: Rc<BTreeTable>) {
        let name = normalize_ident(&table.name);
        self.tables.insert(name, table);
    }

    pub fn get_table(&self, name: &str) -> Option<Rc<BTreeTable>> {
        let name = normalize_ident(name);
        self.tables.get(&name).cloned()
    }
}

pub enum Table {
    BTree(Rc<BTreeTable>),
    Pseudo(Rc<PseudoTable>),
}

impl Table {
    pub fn is_pseudo(&self) -> bool {
        match self {
            Table::Pseudo(_) => true,
            _ => false,
        }
    }

    pub fn column_is_rowid_alias(&self, col: &Column) -> bool {
        match self {
            Table::BTree(table) => table.column_is_rowid_alias(col),
            Table::Pseudo(_) => false,
        }
    }

    pub fn get_column(&self, name: &str) -> Option<(usize, &Column)> {
        match self {
            Table::BTree(table) => table.get_column(name),
            Table::Pseudo(table) => table.get_column(name),
        }
    }

    pub fn columns(&self) -> &Vec<Column> {
        match self {
            Table::BTree(table) => &table.columns,
            Table::Pseudo(table) => &table.columns,
        }
    }
}

pub struct BTreeTable {
    pub root_page: usize,
    pub name: String,
    pub primary_key_column_names: Vec<String>,
    pub columns: Vec<Column>,
    pub has_rowid: bool,
}

impl BTreeTable {
    pub fn column_is_rowid_alias(&self, col: &Column) -> bool {
        let composite_primary_key = self.columns.iter().filter(|col| col.primary_key).count() > 1;
        col.primary_key && col.ty == Type::Integer && !composite_primary_key && self.has_rowid
    }

    pub fn get_column(&self, name: &str) -> Option<(usize, &Column)> {
        let name = normalize_ident(name);
        for (i, column) in self.columns.iter().enumerate() {
            if column.name == name {
                return Some((i, column));
            }
        }
        None
    }

    pub fn from_sql(sql: &str, root_page: usize) -> Result<BTreeTable> {
        let mut parser = Parser::new(sql.as_bytes());
        let cmd = parser.next()?;
        match cmd {
            Some(Cmd::Stmt(Stmt::CreateTable { tbl_name, body, .. })) => {
                create_table(tbl_name, body, root_page)
            }
            _ => anyhow::bail!("Expected CREATE TABLE statement"),
        }
    }

    #[cfg(test)]
    pub fn to_sql(&self) -> String {
        let mut sql = format!("CREATE TABLE {} (\n", self.name);
        for (i, column) in self.columns.iter().enumerate() {
            if i > 0 {
                sql.push_str(",\n");
            }
            sql.push_str("  ");
            sql.push_str(&column.name);
            sql.push(' ');
            sql.push_str(&column.ty.to_string());
        }
        sql.push_str(");\n");
        sql
    }
}

pub struct PseudoTable {
    pub columns: Vec<Column>,
}

impl PseudoTable {
    pub fn new() -> Self {
        Self { columns: vec![] }
    }

    pub fn add_column(&mut self, name: &str, ty: Type, primary_key: bool) {
        self.columns.push(Column {
            name: name.to_string(),
            ty,
            primary_key,
        });
    }
    pub fn get_column(&self, name: &str) -> Option<(usize, &Column)> {
        let name = normalize_ident(name);
        for (i, column) in self.columns.iter().enumerate() {
            if column.name == name {
                return Some((i, column));
            }
        }
        None
    }
}

fn create_table(
    tbl_name: QualifiedName,
    body: CreateTableBody,
    root_page: usize,
) -> Result<BTreeTable> {
    let table_name = normalize_ident(&tbl_name.name.0);
    trace!("Creating table {}", table_name);
    let mut has_rowid = true;
    let mut primary_key_column_names = vec![];
    let mut cols = vec![];
    match body {
        CreateTableBody::ColumnsAndConstraints {
            columns,
            constraints: _,
            options,
        } => {
            for column in columns {
                let name = column.col_name.0.to_string();
                let ty = match column.col_type {
                    Some(data_type) => {
                        let type_name = data_type.name.as_str();
                        if type_name.contains("INTEGER") {
                            Type::Integer
                        } else if type_name.contains("CHAR")
                            || type_name.contains("CLOB")
                            || type_name.contains("TEXT")
                        {
                            Type::Text
                        } else if type_name.contains("BLOB") || type_name.is_empty() {
                            Type::Blob
                        } else if type_name.contains("REAL")
                            || type_name.contains("FLOA")
                            || type_name.contains("DOUB")
                        {
                            Type::Real
                        } else {
                            Type::Numeric
                        }
                    }
                    None => Type::Null,
                };
                let primary_key = column.constraints.iter().any(|c| {
                    matches!(
                        c.constraint,
                        sqlite3_parser::ast::ColumnConstraint::PrimaryKey { .. }
                    )
                });
                cols.push(Column {
                    name,
                    ty,
                    primary_key,
                });
            }
            if options.contains(TableOptions::WITHOUT_ROWID) {
                has_rowid = false;
            }
        }
        CreateTableBody::AsSelect(_) => todo!(),
    };
    Ok(BTreeTable {
        root_page,
        name: table_name,
        has_rowid,
        primary_key_column_names,
        columns: cols,
    })
}

pub fn build_pseudo_table(columns: &[ResultColumn]) -> PseudoTable {
    let table = PseudoTable::new();
    for column in columns {
        match column {
            ResultColumn::Expr(expr, _as_name) => match expr {
                _ => {
                    todo!("unsupported expression {:?}", expr);
                }
            },
            ResultColumn::Star => {
                todo!();
            }
            ResultColumn::TableStar(_) => {
                todo!();
            }
        }
    }
    table
}

pub struct Column {
    pub name: String,
    pub ty: Type,
    pub primary_key: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Type {
    Null,
    Text,
    Numeric,
    Integer,
    Real,
    Blob,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Type::Null => "NULL",
            Type::Text => "TEXT",
            Type::Numeric => "NUMERIC",
            Type::Integer => "INTEGER",
            Type::Real => "REAL",
            Type::Blob => "BLOB",
        };
        write!(f, "{}", s)
    }
}

pub fn sqlite_schema_table() -> BTreeTable {
    BTreeTable {
        root_page: 1,
        name: "sqlite_schema".to_string(),
        has_rowid: true,
        primary_key_column_names: vec![],
        columns: vec![
            Column {
                name: "type".to_string(),
                ty: Type::Text,
                primary_key: false,
            },
            Column {
                name: "name".to_string(),
                ty: Type::Text,
                primary_key: false,
            },
            Column {
                name: "tbl_name".to_string(),
                ty: Type::Text,
                primary_key: false,
            },
            Column {
                name: "rootpage".to_string(),
                ty: Type::Integer,
                primary_key: false,
            },
            Column {
                name: "sql".to_string(),
                ty: Type::Text,
                primary_key: false,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_has_rowid_true() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER PRIMARY KEY, b TEXT);"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        assert!(table.has_rowid, "has_rowid should be set to true");
        Ok(())
    }

    #[test]
    pub fn test_has_rowid_false() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER PRIMARY KEY, b TEXT) WITHOUT ROWID;"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        assert!(!table.has_rowid, "has_rowid should be set to false");
        Ok(())
    }

    #[test]
    pub fn test_column_is_rowid_alias_single_text() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a TEXT PRIMARY KEY, b TEXT);"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            !table.column_is_rowid_alias(column),
            "column 'a´ has type different than INTEGER so can't be a rowid alias"
        );
        Ok(())
    }

    #[test]
    pub fn test_column_is_rowid_alias_single_integer() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER PRIMARY KEY, b TEXT);"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            table.column_is_rowid_alias(column),
            "column 'a´ should be a rowid alias"
        );
        Ok(())
    }

    #[test]
    #[ignore] // We don't support separate definition of primary keys yet
    pub fn test_column_is_rowid_alias_single_integer_separate_primary_key_definition() -> Result<()>
    {
        let sql = r#"CREATE TABLE t1 (a INTEGER, b TEXT, PRIMARY KEY(a));"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            table.column_is_rowid_alias(column),
            "column 'a´ should be a rowid alias"
        );
        Ok(())
    }

    #[test]
    pub fn test_column_is_rowid_alias_single_integer_separate_primary_key_definition_without_rowid(
    ) -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER, b TEXT, PRIMARY KEY(a)) WITHOUT ROWID;"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            !table.column_is_rowid_alias(column),
            "column 'a´ shouldn't be a rowid alias because table has no rowid"
        );
        Ok(())
    }

    #[test]
    pub fn test_column_is_rowid_alias_single_integer_without_rowid() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER PRIMARY KEY, b TEXT) WITHOUT ROWID;"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            !table.column_is_rowid_alias(column),
            "column 'a´ shouldn't be a rowid alias because table has no rowid"
        );
        Ok(())
    }

    #[test]
    pub fn test_column_is_rowid_alias_inline_composite_primary_key() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER PRIMARY KEY, b TEXT PRIMARY KEY);"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            !table.column_is_rowid_alias(column),
            "column 'a´ shouldn't be a rowid alias because table has composite primary key"
        );
        Ok(())
    }

    #[test]
    pub fn test_column_is_rowid_alias_separate_composite_primary_key_definition() -> Result<()> {
        let sql = r#"CREATE TABLE t1 (a INTEGER, b TEXT, PRIMARY KEY(a, b));"#;
        let table = BTreeTable::from_sql(sql, 0)?;
        let column = table.get_column("a").unwrap().1;
        assert!(
            !table.column_is_rowid_alias(column),
            "column 'a´ shouldn't be a rowid alias because table has composite primary key"
        );
        Ok(())
    }

    #[test]
    pub fn test_sqlite_schema() {
        let expected = r#"CREATE TABLE sqlite_schema (
  type TEXT,
  name TEXT,
  tbl_name TEXT,
  rootpage INTEGER,
  sql TEXT);
"#;
        let actual = sqlite_schema_table().to_sql();
        assert_eq!(expected, actual);
    }
}
