//! Pure string parsing of the method-name DSL. No syn types — unit-testable.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Op {
    Eq,
    In,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    IsNull,
    IsNotNull,
}

impl Op {
    pub fn takes_param(self) -> bool {
        !matches!(self, Op::IsNull | Op::IsNotNull)
    }

    pub fn is_list(self) -> bool {
        matches!(self, Op::In)
    }

    /// SQL fragment up to (not including) the placeholder. For `IsNull`/`IsNotNull` this
    /// is the complete condition (no placeholder follows). For `In` this is
    /// `"{target} IN "` — the caller appends a QueryBuilder-rendered value list, since no
    /// backend lets a single placeholder bind a `Vec<T>` the way this crate used to rely
    /// on Postgres's `= ANY($n)`.
    pub fn prefix(self, target: &str) -> String {
        match self {
            Op::Eq => format!("{target} = "),
            Op::In => format!("{target} IN "),
            Op::Gt => format!("{target} > "),
            Op::Gte => format!("{target} >= "),
            Op::Lt => format!("{target} < "),
            Op::Lte => format!("{target} <= "),
            Op::Like => format!("{target} LIKE "),
            Op::IsNull => format!("{target} IS NULL"),
            Op::IsNotNull => format!("{target} IS NOT NULL"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Connector {
    And,
    Or,
}

impl Connector {
    fn sql(self) -> &'static str {
        match self {
            Connector::And => "AND",
            Connector::Or => "OR",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Condition {
    pub column: String,
    pub op: Op,
    /// Connector between this condition and the next one.
    pub connector: Option<Connector>,
}

// Longest suffixes first so `_gte` wins over `_gt`, `_is_not_null` over `_is_null`.
const OP_SUFFIXES: &[(&str, Op)] = &[
    ("_is_not_null", Op::IsNotNull),
    ("_is_null", Op::IsNull),
    ("_like", Op::Like),
    ("_gte", Op::Gte),
    ("_lte", Op::Lte),
    ("_gt", Op::Gt),
    ("_lt", Op::Lt),
    ("_in", Op::In),
];

fn split_op(field: &str) -> (&str, Op) {
    for (suffix, op) in OP_SUFFIXES {
        if let Some(col) = field.strip_suffix(suffix) {
            if !col.is_empty() {
                return (col, *op);
            }
        }
    }
    (field, Op::Eq)
}

/// "username_or_email" → [username OR][email], "a_and_b_in" → [a AND][b IN], "x_is_null" → [x IS NULL]
pub fn parse_conditions(s: &str) -> Result<Vec<Condition>, String> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        let and_pos = rest.find("_and_");
        let or_pos = rest.find("_or_");
        let (field, connector, next) = match (and_pos, or_pos) {
            (None, None) => (rest, None, None),
            (Some(a), None) => (&rest[..a], Some(Connector::And), Some(&rest[a + 5..])),
            (None, Some(o)) => (&rest[..o], Some(Connector::Or), Some(&rest[o + 4..])),
            (Some(a), Some(o)) if a < o => (&rest[..a], Some(Connector::And), Some(&rest[a + 5..])),
            (_, Some(o)) => (&rest[..o], Some(Connector::Or), Some(&rest[o + 4..])),
        };
        let (column, op) = split_op(field);
        if column.is_empty() {
            return Err(format!("empty column name in filter `{s}`"));
        }
        out.push(Condition { column: column.to_string(), op, connector });
        match next {
            Some(n) => rest = n,
            None => break,
        }
    }
    Ok(out)
}

/// One piece of a WHERE clause, in the shape needed to emit either a static SQL string
/// (the fast path, when no condition uses `_in`) or a `sqlx::QueryBuilder` sequence （the
/// path taken as soon as any condition does — see `WhereClause::has_in`).
#[derive(Debug, Clone)]
pub enum WhereChunk {
    /// Raw SQL text (column/operator text, connectors, or a complete no-param condition).
    Literal(String),
    /// A regular scalar bind — index into the caller's WHERE-relevant bind expressions.
    Bind(usize),
    /// An `_in` bind (a `Vec<T>`) — same indexing as `Bind`, rendered via
    /// `axumstart_db::push_in_list` instead of a single placeholder.
    InList(usize),
}

pub struct WhereClause {
    /// Only valid when `has_in` is false — pre-rendered with dialect placeholders.
    pub sql: String,
    /// Always populated; used by the QueryBuilder codegen path when `has_in` is true.
    pub chunks: Vec<WhereChunk>,
    /// True if any condition uses the `_in` operator, forcing the QueryBuilder path.
    pub has_in: bool,
    /// Join table names referenced by conditions (in first-use order).
    pub joins_needed: Vec<String>,
    /// Base-table columns usable for Row-struct probing (join-resolved columns excluded).
    pub probe_cols: Vec<String>,
    /// Number of bind parameters the WHERE clause consumes (one per `In` too — it still
    /// binds a single `Vec<T>` method parameter).
    pub params: usize,
}

/// `sql_offset` shifts placeholder indices (UPDATE puts SET params first) in the fast-path
/// `sql` string. `bind_offset` is separate: it indexes into whatever slice of bind
/// expressions the caller passes alongside `chunks` (e.g. `gen_set` passes only the
/// filter-side binds, not the SET-side ones), so it usually stays 0.
pub fn build_where(
    conds: &[Condition],
    joins: &[String],
    sql_offset: usize,
    bind_offset: usize,
    placeholder: &dyn Fn(usize) -> String,
) -> WhereClause {
    let mut joins_needed: Vec<String> = Vec::new();
    let mut probe_cols: Vec<String> = Vec::new();
    let mut sql_parts: Vec<String> = Vec::new();
    let mut chunks: Vec<WhereChunk> = Vec::new();
    let mut has_in = false;
    let mut idx = sql_offset;
    let mut bind_idx = bind_offset;

    for c in conds {
        let target = resolve_target(&c.column, joins, &mut joins_needed, &mut probe_cols);
        let prefix = c.op.prefix(&target);

        if c.op.is_list() {
            has_in = true;
            idx += 1;
            chunks.push(WhereChunk::Literal(prefix));
            chunks.push(WhereChunk::InList(bind_idx));
            bind_idx += 1;
        } else if c.op.takes_param() {
            idx += 1;
            let ph = placeholder(idx);
            sql_parts.push(format!("{prefix}{ph}"));
            chunks.push(WhereChunk::Literal(prefix));
            chunks.push(WhereChunk::Bind(bind_idx));
            bind_idx += 1;
        } else {
            sql_parts.push(prefix.clone());
            chunks.push(WhereChunk::Literal(prefix));
        }

        if let Some(conn) = c.connector {
            let conn_sql = format!(" {} ", conn.sql());
            sql_parts.push(conn.sql().to_string());
            chunks.push(WhereChunk::Literal(conn_sql));
        }
    }

    WhereClause {
        sql: sql_parts.join(" "),
        chunks,
        has_in,
        joins_needed,
        probe_cols,
        params: idx - sql_offset,
    }
}

fn resolve_target(
    column: &str,
    joins: &[String],
    joins_needed: &mut Vec<String>,
    probe_cols: &mut Vec<String>,
) -> String {
    for j in joins {
        let prefix = format!("{j}_");
        if let Some(col) = column.strip_prefix(prefix.as_str()) {
            // "{join}_id" is the FK column itself — query it directly, no JOIN needed
            if col == "id" {
                break;
            }
            if !joins_needed.contains(j) {
                joins_needed.push(j.clone());
            }
            return format!("\"{j}\".{col}");
        }
    }
    probe_cols.push(column.to_string());
    column.to_string()
}

pub fn join_clauses(table: &str, needed: &[String]) -> String {
    needed
        .iter()
        .map(|j| format!("JOIN \"{j}\" ON \"{j}\".id = \"{table}\".{j}_id"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// "col_desc" → ("col DESC", "col"), "col_asc" → ("col ASC", "col"), "col" → ("col", "col")
pub fn format_order_col(s: &str) -> (String, String) {
    if let Some(col) = s.strip_suffix("_desc") {
        (format!("{col} DESC"), col.to_string())
    } else if let Some(col) = s.strip_suffix("_asc") {
        (format!("{col} ASC"), col.to_string())
    } else {
        (s.to_string(), s.to_string())
    }
}

/// Splits "filter_part_order_by_col_desc" → ("filter_part", Some(("ORDER BY col DESC", "col")))
pub fn split_order(field_str: &str) -> (&str, Option<(String, String)>) {
    if let Some(pos) = field_str.find("_order_by_") {
        let filter = &field_str[..pos];
        let order = &field_str[pos + "_order_by_".len()..];
        let (rendered, col) = format_order_col(order);
        (filter, Some((format!("ORDER BY {rendered}"), col)))
    } else {
        (field_str, None)
    }
}

/// Strips "_this_week" suffix → (remaining, true) or (original, false)
pub fn split_this_week(field_str: &str) -> (&str, bool) {
    match field_str.strip_suffix("_this_week") {
        Some(rest) => (rest, true),
        None => (field_str, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(col: &str, op: Op, connector: Option<Connector>) -> Condition {
        Condition { column: col.to_string(), op, connector }
    }

    fn pg_placeholder(idx: usize) -> String {
        format!("${idx}")
    }

    #[test]
    fn plain_field() {
        assert_eq!(parse_conditions("user_id").unwrap(), vec![cond("user_id", Op::Eq, None)]);
    }

    #[test]
    fn and_or_chain() {
        assert_eq!(
            parse_conditions("username_or_email").unwrap(),
            vec![cond("username", Op::Eq, Some(Connector::Or)), cond("email", Op::Eq, None)]
        );
        assert_eq!(
            parse_conditions("challenge_id_and_user_id").unwrap(),
            vec![
                cond("challenge_id", Op::Eq, Some(Connector::And)),
                cond("user_id", Op::Eq, None)
            ]
        );
    }

    #[test]
    fn op_suffixes() {
        assert_eq!(parse_conditions("id_in").unwrap(), vec![cond("id", Op::In, None)]);
        assert_eq!(parse_conditions("elo_gte").unwrap(), vec![cond("elo", Op::Gte, None)]);
        assert_eq!(parse_conditions("elo_gt").unwrap(), vec![cond("elo", Op::Gt, None)]);
        assert_eq!(parse_conditions("name_like").unwrap(), vec![cond("name", Op::Like, None)]);
        assert_eq!(
            parse_conditions("deleted_at_is_null").unwrap(),
            vec![cond("deleted_at", Op::IsNull, None)]
        );
        assert_eq!(
            parse_conditions("deleted_at_is_not_null").unwrap(),
            vec![cond("deleted_at", Op::IsNotNull, None)]
        );
    }

    #[test]
    fn mixed_ops_and_connectors() {
        assert_eq!(
            parse_conditions("user_id_and_deleted_at_is_null").unwrap(),
            vec![
                cond("user_id", Op::Eq, Some(Connector::And)),
                cond("deleted_at", Op::IsNull, None)
            ]
        );
        assert_eq!(
            parse_conditions("id_in_and_elo_gt").unwrap(),
            vec![cond("id", Op::In, Some(Connector::And)), cond("elo", Op::Gt, None)]
        );
    }

    #[test]
    fn where_placeholders_skip_no_param_ops() {
        let conds = parse_conditions("user_id_and_deleted_at_is_null_and_elo_gt").unwrap();
        let wc = build_where(&conds, &[], 0, 0, &pg_placeholder);
        assert_eq!(wc.sql, "user_id = $1 AND deleted_at IS NULL AND elo > $2");
        assert_eq!(wc.params, 2);
        assert_eq!(wc.probe_cols, vec!["user_id", "deleted_at", "elo"]);
        assert!(!wc.has_in);
    }

    #[test]
    fn where_with_offset() {
        let conds = parse_conditions("id").unwrap();
        let wc = build_where(&conds, &[], 2, 0, &pg_placeholder);
        assert_eq!(wc.sql, "id = $3");
        assert_eq!(wc.params, 1);
    }

    #[test]
    fn join_resolution() {
        let joins = vec!["user".to_string()];
        let conds = parse_conditions("user_email_and_user_id").unwrap();
        let wc = build_where(&conds, &joins, 0, 0, &pg_placeholder);
        assert_eq!(wc.sql, "\"user\".email = $1 AND user_id = $2");
        assert_eq!(wc.joins_needed, vec!["user"]);
        // joined column not probed; FK shortcut column is
        assert_eq!(wc.probe_cols, vec!["user_id"]);
    }

    #[test]
    fn order_and_week_suffixes() {
        let (filter, order) = split_order("user_id_this_week_order_by_created_at_desc");
        assert_eq!(filter, "user_id_this_week");
        let (order_sql, order_col) = order.unwrap();
        assert_eq!(order_sql, "ORDER BY created_at DESC");
        assert_eq!(order_col, "created_at");
        let (filter, week) = split_this_week(filter);
        assert_eq!(filter, "user_id");
        assert!(week);
    }

    #[test]
    fn empty_column_is_error() {
        assert!(parse_conditions("_and_x").is_err());
        assert!(parse_conditions("x_and_").is_err());
    }

    #[test]
    fn in_condition_sets_has_in_and_chunks() {
        let conds = parse_conditions("id_in_and_elo_gt").unwrap();
        let wc = build_where(&conds, &[], 0, 0, &pg_placeholder);
        assert!(wc.has_in);
        assert_eq!(wc.params, 2);
        match &wc.chunks[..] {
            [
                WhereChunk::Literal(l0),
                WhereChunk::InList(0),
                WhereChunk::Literal(conn),
                WhereChunk::Literal(l1),
                WhereChunk::Bind(1),
            ] => {
                assert_eq!(l0, "id IN ");
                assert_eq!(conn, " AND ");
                assert_eq!(l1, "elo > ");
            }
            other => panic!("unexpected chunk shape: {other:?}"),
        }
    }
}
