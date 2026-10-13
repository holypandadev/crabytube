use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrowFunctionExpression, AssignmentOperator, AssignmentTarget, BindingPattern,
    CallExpression, Expression, Function, Statement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::{operator::LogicalOperator, scope::ScopeFlags};

const INTL_POLYFILL: &str = include_str!("polyfill/intl.js");
const SETUP_CODE: &str = include_str!("polyfill/setup.js");

pub fn preprocess_player(source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::mjs()).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(format!(
            "OXC parser diagnostics: {}",
            parsed.diagnostics.len()
        ));
    }
    let program = &parsed.program;
    let main_statement = program
        .body
        .last()
        .ok_or_else(|| "player module is empty".to_owned())?;
    let function =
        main_iife(main_statement).ok_or_else(|| "unexpected player IIFE structure".to_owned())?;
    let body = function
        .body
        .as_deref()
        .ok_or_else(|| "player IIFE has no body".to_owned())?;
    let skip_first = usize::from(program.body.len() == 2);
    let statements = body.statements.iter().skip(skip_first);

    let mut solvers = Vec::new();
    for statement in body.statements.iter().skip(skip_first) {
        if !should_keep(statement) {
            continue;
        }
        for (name, function) in function_infos(source, statement) {
            if has_alr_yes_pattern(function) {
                solvers.push(name);
            }
        }
    }
    if solvers.is_empty() {
        return Err("found 0 solver functions".into());
    }

    let first_body_statement = statements
        .clone()
        .next()
        .map(GetSpan::span)
        .map(|span| span.start as usize)
        .unwrap_or(body.span.end as usize - 1);
    let mut output =
        String::with_capacity(source.len() / 4 + INTL_POLYFILL.len() + SETUP_CODE.len());
    output.push_str(INTL_POLYFILL);
    output.push('\n');
    output.push_str(SETUP_CODE);
    output.push('\n');

    if program.body.len() == 2 {
        let prelude = program.body[0].span();
        output.push_str(source_slice(
            source,
            prelude.start as usize,
            prelude.end as usize,
        )?);
        output.push('\n');
    }
    let outer_span = main_statement.span();
    let body_open_end = body.span.start as usize + 1;
    output.push_str(source_slice(
        source,
        outer_span.start as usize,
        body_open_end,
    )?);
    if skip_first == 0 {
        output.push_str(source_slice(source, body_open_end, first_body_statement)?);
    }

    for statement in body.statements.iter().skip(skip_first) {
        if should_keep(statement) {
            output.push_str(&statement_code(source, statement)?);
            output.push('\n');
        }
    }
    output.push_str("_result.n = ");
    output.push_str(&multi_try(&solvers, "n"));
    output.push_str(";\n_result.sig = ");
    output.push_str(&multi_try(&solvers, "sig"));
    output.push_str(";\n");

    let body_close = body.span.end as usize - 1;
    output.push_str(source_slice(source, body_close, outer_span.end as usize)?);
    Ok(output)
}

fn main_iife<'a>(statement: &'a Statement<'a>) -> Option<&'a Function<'a>> {
    let Statement::ExpressionStatement(expression_statement) = statement else {
        return None;
    };
    let Expression::CallExpression(call) = &expression_statement.expression else {
        return None;
    };
    function_expression(&call.callee)
}

fn function_expression<'a>(expression: &'a Expression<'a>) -> Option<&'a Function<'a>> {
    match expression {
        Expression::FunctionExpression(function) => Some(function),
        Expression::ParenthesizedExpression(paren) => function_expression(&paren.expression),
        Expression::StaticMemberExpression(member) => function_expression(&member.object),
        Expression::ComputedMemberExpression(member) => function_expression(&member.object),
        _ => None,
    }
}

fn should_keep(statement: &Statement<'_>) -> bool {
    let Statement::ExpressionStatement(expression_statement) = statement else {
        return true;
    };
    matches!(
        &expression_statement.expression,
        Expression::AssignmentExpression(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::StringLiteral(_)
    )
}

fn function_infos<'a>(
    source: &str,
    statement: &'a Statement<'a>,
) -> Vec<(String, &'a Function<'a>)> {
    match statement {
        Statement::FunctionDeclaration(function) => {
            let Some(name) = &function.id else {
                return Vec::new();
            };
            vec![(name.name.to_string(), function)]
        }
        Statement::ExpressionStatement(expression_statement) => {
            let Expression::AssignmentExpression(assignment) = &expression_statement.expression
            else {
                return Vec::new();
            };
            if assignment.operator != AssignmentOperator::Assign {
                return Vec::new();
            }
            let Expression::FunctionExpression(function) = &assignment.right else {
                return Vec::new();
            };
            vec![(assignment_name(source, &assignment.left), function)]
        }
        Statement::VariableDeclaration(declaration) => declaration
            .declarations
            .iter()
            .filter_map(|declarator| {
                let Expression::FunctionExpression(function) = declarator.init.as_ref()? else {
                    return None;
                };
                let name = match &declarator.id {
                    BindingPattern::BindingIdentifier(identifier) => identifier.name.to_string(),
                    _ => "undefined".to_owned(),
                };
                Some((name, function.as_ref()))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn assignment_name(source: &str, target: &AssignmentTarget<'_>) -> String {
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => identifier.name.to_string(),
        AssignmentTarget::StaticMemberExpression(_)
        | AssignmentTarget::ComputedMemberExpression(_) => source_slice(
            source,
            target.span().start as usize,
            target.span().end as usize,
        )
        .unwrap_or("undefined")
        .to_owned(),
        _ => "undefined".to_owned(),
    }
}

fn has_alr_yes_pattern(function: &Function<'_>) -> bool {
    let Some(body) = function.body.as_deref() else {
        return false;
    };
    let mut finder = AlrYesFinder { found: false };
    finder.visit_function_body(body);
    finder.found
}

struct AlrYesFinder {
    found: bool,
}

impl<'a> Visit<'a> for AlrYesFinder {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if is_alr_yes_call(call) {
            self.found = true;
        } else if !self.found {
            walk::walk_call_expression(self, call);
        }
    }

    fn visit_function(&mut self, _function: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _expression: &ArrowFunctionExpression<'a>) {}
}

fn is_alr_yes_call(call: &CallExpression<'_>) -> bool {
    let Expression::StaticMemberExpression(_) = &call.callee else {
        return false;
    };
    call.arguments.len() >= 2
        && is_string_argument(&call.arguments[0], "alr")
        && is_string_argument(&call.arguments[1], "yes")
}

fn is_string_argument(argument: &Argument<'_>, expected: &str) -> bool {
    matches!(argument, Argument::StringLiteral(string) if string.value.as_str() == expected)
}

fn statement_code(source: &str, statement: &Statement<'_>) -> Result<String, String> {
    if let Statement::FunctionDeclaration(function) = statement
        && function
            .id
            .as_ref()
            .is_some_and(|identifier| identifier.name.as_str() == "g")
    {
        let function_name = function.id.as_ref().unwrap();
        let span = statement.span();
        let after_name = function_name.span.end as usize;
        let rest = source_slice(source, after_name, span.end as usize)?;
        let async_prefix = if function.r#async {
            "g = async function"
        } else {
            "g = function"
        };
        return Ok(format!("{async_prefix}{rest};"));
    }
    if let Statement::ExpressionStatement(expression_statement) = statement
        && let Expression::AssignmentExpression(assignment) = &expression_statement.expression
        && let Expression::LogicalExpression(logical) = &assignment.right
        && logical.operator == LogicalOperator::Or
        && matches!(&logical.left, Expression::ThisExpression(_))
        && matches!(&logical.right, Expression::Identifier(identifier) if identifier.name.as_str() == "self")
    {
        let span = statement.span();
        return Ok(format!(
            "{}{}{}",
            source_slice(source, span.start as usize, logical.span.start as usize)?,
            source_slice(
                source,
                logical.right.span().start as usize,
                logical.right.span().end as usize
            )?,
            source_slice(source, logical.span.end as usize, span.end as usize)?,
        ));
    }
    let span = statement.span();
    Ok(source_slice(source, span.start as usize, span.end as usize)?.to_owned())
}

fn multi_try(solvers: &[String], kind: &str) -> String {
    let wrappers: Vec<String> = solvers
        .iter()
        .map(|name| {
            let solver = format!(
                r#"({{sig, n}}) => {{
  const url = ({name})("https://youtube.com/watch?v=yt-dlp-wins", "s", sig ? encodeURIComponent(sig) : undefined);
  url.set("n", n);
  const proto = Object.getPrototypeOf(url);
  const keys = Object.keys(proto).concat(Object.getOwnPropertyNames(proto));
  for (const key of keys) {{
    if (!["constructor", "set", "get", "clone"].includes(key)) {{
      url[key]();
      break;
    }}
  }}
  const s = url.get("s");
  return {{sig: s ? decodeURIComponent(s) : null, n: url.get("n") ?? null}};
}}"#
            );
            format!("({kind}) => ({solver})({{ {kind} }}).{kind}")
        })
        .collect();
    match wrappers.as_slice() {
        [] => "(_input) => { throw 'no solutions'; }".to_owned(),
        [single] => single.clone(),
        _ => format!(
            r#"(_input) => {{
  const _results = new Set();
  const errors = [];
  for (const _generator of [{}]) {{
    try {{ _results.add(_generator(_input)); }} catch (e) {{ errors.push(e); }}
  }}
  if (!_results.size) throw "no solutions: " + errors.join(", ");
  if (_results.size !== 1) throw "invalid solutions: " + _results.size + " different results";
  return _results.values().next().value;
}}"#,
            wrappers.join(", ")
        ),
    }
}

fn source_slice(source: &str, start: usize, end: usize) -> Result<&str, String> {
    source
        .get(start..end)
        .ok_or_else(|| format!("invalid OXC source span {start}..{end}"))
}

#[cfg(test)]
mod tests {
    use super::preprocess_player;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    #[test]
    fn preprocesses_solver_and_emits_parseable_result_assignments() {
        let source = r#"(function() {
  function sig(a, b, c) {
    window.check("alr", "yes");
    return new URLSearchParams("s=encoded&n=transformed");
  }
})();"#;

        let prepared = preprocess_player(source).expect("player should preprocess");
        assert!(prepared.contains("function sig"));
        assert!(prepared.contains("_result.n ="));
        assert!(prepared.contains("_result.sig ="));

        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &prepared, SourceType::mjs()).parse();
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    }

    #[test]
    fn rejects_player_modules_without_a_solver() {
        let source = "(function() { function unrelated() { return 1; } })();";
        assert!(preprocess_player(source).is_err());
    }
}
