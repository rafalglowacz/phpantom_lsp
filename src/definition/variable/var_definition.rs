/// Variable definition search in AST.
///
/// This submodule contains the AST walk that finds the definition site
/// of a `$variable` reference: assignments, parameters, foreach
/// key/value, catch variables, static/global declarations, and
/// array/list destructuring.
///
/// The entry point is [`find_variable_definition_in_program`], called
/// from the `Backend` methods in the parent `variable` module.
use mago_span::HasSpan;
use mago_syntax::ast::sequence::TokenSeparatedSequence;
use mago_syntax::ast::*;

use super::VarDefSearchResult;

/// Represents a definition site found during a statement walk.
#[derive(Clone, Copy)]
pub(super) struct DefSite {
    /// Byte offset of the `$var` token start.
    pub(super) offset: u32,
    /// Byte offset of the `$var` token end.
    pub(super) end_offset: u32,
}

/// Result of checking an expression for a variable definition.
pub(super) enum ExprDefResult {
    /// The cursor is on the definition.
    AtDefinition,
    /// Found a definition site.
    Found(DefSite),
}

/// Top-level entry: find the definition site of `var_name` in the parsed
/// program at the given cursor offset.
pub(super) fn find_variable_definition_in_program(
    program: &Program<'_>,
    _content: &str,
    var_name: &str,
    cursor_offset: u32,
) -> VarDefSearchResult {
    // Walk top-level statements, drilling into the scope that contains
    // the cursor.
    find_in_statements(program.statements.iter(), var_name, cursor_offset)
}

/// Walk a sequence of statements looking for the scope that contains the
/// cursor, then search within that scope for the variable definition.
fn find_in_statements<'a, I>(
    statements: I,
    var_name: &str,
    cursor_offset: u32,
) -> VarDefSearchResult
where
    I: Iterator<Item = &'a Statement<'a>>,
{
    let stmts: Vec<&Statement> = statements.collect();

    // Step 1: Check if the cursor is inside a class, function, or namespace.
    for &stmt in &stmts {
        match stmt {
            Statement::Class(class) => {
                let start = class.left_brace.start.offset;
                let end = class.right_brace.end.offset;
                if cursor_offset >= start && cursor_offset <= end {
                    return find_in_class_members(class.members.iter(), var_name, cursor_offset);
                }
            }
            Statement::Interface(iface) => {
                let start = iface.left_brace.start.offset;
                let end = iface.right_brace.end.offset;
                if cursor_offset >= start && cursor_offset <= end {
                    return find_in_class_members(iface.members.iter(), var_name, cursor_offset);
                }
            }
            Statement::Trait(trait_def) => {
                let start = trait_def.left_brace.start.offset;
                let end = trait_def.right_brace.end.offset;
                if cursor_offset >= start && cursor_offset <= end {
                    return find_in_class_members(
                        trait_def.members.iter(),
                        var_name,
                        cursor_offset,
                    );
                }
            }
            Statement::Enum(enum_def) => {
                let start = enum_def.left_brace.start.offset;
                let end = enum_def.right_brace.end.offset;
                if cursor_offset >= start && cursor_offset <= end {
                    return find_in_class_members(enum_def.members.iter(), var_name, cursor_offset);
                }
            }
            Statement::Namespace(ns) => {
                // Recurse into namespace body.
                let result = find_in_statements(ns.statements().iter(), var_name, cursor_offset);
                if !matches!(result, VarDefSearchResult::NotFound) {
                    return result;
                }
            }
            Statement::Function(func) => {
                let body_start = func.body.left_brace.start.offset;
                let body_end = func.body.right_brace.end.offset;
                if cursor_offset >= body_start && cursor_offset <= body_end {
                    return find_in_function_scope(
                        &func.parameter_list,
                        func.body.statements.iter(),
                        var_name,
                        cursor_offset,
                    );
                }
            }
            _ => {}
        }
    }

    // Step 2: Cursor is in top-level code.  Walk all statements.
    find_def_in_statement_list(&stmts, var_name, cursor_offset, None)
}

/// Search class-like members (methods) for the scope containing the cursor.
fn find_in_class_members<'a, I>(
    members: I,
    var_name: &str,
    cursor_offset: u32,
) -> VarDefSearchResult
where
    I: Iterator<Item = &'a ClassLikeMember<'a>>,
{
    for member in members {
        if let ClassLikeMember::Method(method) = member
            && let MethodBody::Concrete(body) = &method.body
        {
            let body_start = body.left_brace.start.offset;
            let body_end = body.right_brace.end.offset;
            if cursor_offset >= body_start && cursor_offset <= body_end {
                return find_in_function_scope(
                    &method.parameter_list,
                    body.statements.iter(),
                    var_name,
                    cursor_offset,
                );
            }
        }
    }
    VarDefSearchResult::NotFound
}

/// Search within a function/method scope: check parameters first, then
/// walk the body statements.
fn find_in_function_scope<'a, I>(
    params: &FunctionLikeParameterList<'a>,
    body_statements: I,
    var_name: &str,
    cursor_offset: u32,
) -> VarDefSearchResult
where
    I: Iterator<Item = &'a Statement<'a>>,
{
    let stmts: Vec<&Statement> = body_statements.collect();

    // Check if the cursor is inside a nested closure/arrow function.
    if let Some(result) = find_in_nested_closure(&stmts, var_name, cursor_offset) {
        return result;
    }

    // Search body statements for definition sites.
    let body_result = find_def_in_statement_list(&stmts, var_name, cursor_offset, None);
    if !matches!(body_result, VarDefSearchResult::NotFound) {
        return body_result;
    }

    // Check function parameters (searched last because they precede
    // all body statements — if a body assignment exists, it's more
    // recent and takes priority).
    find_in_params(params, var_name, cursor_offset)
}

/// Check if the cursor is inside a closure or arrow function nested
/// within the given statements.  If so, resolve within that inner scope.
fn find_in_nested_closure(
    stmts: &[&Statement<'_>],
    var_name: &str,
    cursor_offset: u32,
) -> Option<VarDefSearchResult> {
    for &stmt in stmts {
        let stmt_span = stmt.span();
        if cursor_offset < stmt_span.start.offset || cursor_offset > stmt_span.end.offset {
            continue;
        }
        if let Some(result) = find_closure_in_statement(stmt, var_name, cursor_offset) {
            return Some(result);
        }
    }
    None
}

/// Recursively check a statement for closures/arrow functions that
/// contain the cursor.
fn find_closure_in_statement(
    stmt: &Statement<'_>,
    var_name: &str,
    cursor_offset: u32,
) -> Option<VarDefSearchResult> {
    match stmt {
        Statement::Expression(expr_stmt) => {
            find_closure_in_expression(expr_stmt.expression, var_name, cursor_offset)
        }
        Statement::Return(ret) => {
            if let Some(expr) = ret.value {
                find_closure_in_expression(expr, var_name, cursor_offset)
            } else {
                None
            }
        }
        Statement::If(if_stmt) => {
            // Check condition
            if let Some(r) = find_closure_in_expression(if_stmt.condition, var_name, cursor_offset)
            {
                return Some(r);
            }
            // Check body statements
            for inner in if_stmt.body.statements() {
                if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            None
        }
        Statement::Foreach(foreach) => {
            for inner in foreach.body.statements() {
                if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            None
        }
        Statement::While(while_stmt) => {
            for inner in while_stmt.body.statements() {
                if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            None
        }
        Statement::For(for_stmt) => {
            for inner in for_stmt.body.statements() {
                if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            None
        }
        Statement::Try(try_stmt) => {
            for inner in try_stmt.block.statements.iter() {
                if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            for catch in try_stmt.catch_clauses.iter() {
                for inner in catch.block.statements.iter() {
                    if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                        return Some(r);
                    }
                }
            }
            if let Some(ref finally) = try_stmt.finally_clause {
                for inner in finally.block.statements.iter() {
                    if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                        return Some(r);
                    }
                }
            }
            None
        }
        Statement::Block(block) => {
            for inner in block.statements.iter() {
                if let Some(r) = find_closure_in_statement(inner, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            None
        }
        _ => None,
    }
}

/// Recursively check an expression for closures/arrow functions that
/// contain the cursor.
fn find_closure_in_expression(
    expr: &Expression<'_>,
    var_name: &str,
    cursor_offset: u32,
) -> Option<VarDefSearchResult> {
    // Quick span check: if cursor is not inside this expression, skip.
    let span = expr.span();
    if cursor_offset < span.start.offset || cursor_offset > span.end.offset {
        return None;
    }

    match expr {
        Expression::Closure(closure) => {
            let body_start = closure.body.left_brace.start.offset;
            let body_end = closure.body.right_brace.end.offset;
            if cursor_offset >= body_start && cursor_offset <= body_end {
                let result = find_in_function_scope(
                    &closure.parameter_list,
                    closure.body.statements.iter(),
                    var_name,
                    cursor_offset,
                );
                if !matches!(result, VarDefSearchResult::NotFound) {
                    return Some(result);
                }
                // Variable not found in the closure's own scope.
                // Check the `use ($var)` clause — captured variables
                // act as definitions visible inside the closure body.
                if let Some(ref use_clause) = closure.use_clause {
                    for use_var in use_clause.variables.iter() {
                        if use_var.variable.name == var_name {
                            let var_start = use_var.variable.span.start.offset;
                            let var_end = use_var.variable.span.end.offset;
                            return Some(VarDefSearchResult::FoundAt {
                                offset: var_start,
                                end_offset: var_end,
                            });
                        }
                    }
                }
                return Some(VarDefSearchResult::NotFound);
            }
            // Check if the cursor is on a variable in the `use` clause
            // itself — treat it as a definition site so the outer-scope
            // lookup can take over.
            if let Some(ref use_clause) = closure.use_clause {
                for use_var in use_clause.variables.iter() {
                    if use_var.variable.name == var_name {
                        let var_start = use_var.variable.span.start.offset;
                        let var_end = use_var.variable.span.end.offset;
                        if cursor_offset >= var_start && cursor_offset < var_end {
                            return Some(VarDefSearchResult::AtDefinition);
                        }
                    }
                }
            }
            None
        }
        Expression::ArrowFunction(arrow) => {
            // Arrow functions have a single expression body.
            // The scope includes parameters.
            let body_span = arrow.expression.span();
            if cursor_offset >= body_span.start.offset && cursor_offset <= body_span.end.offset {
                // Check parameters first.
                let param_result = find_in_params(&arrow.parameter_list, var_name, cursor_offset);
                if !matches!(param_result, VarDefSearchResult::NotFound) {
                    return Some(param_result);
                }
            }
            // Recurse into the body expression.
            find_closure_in_expression(arrow.expression, var_name, cursor_offset)
        }
        Expression::Assignment(assignment) => {
            if let Some(r) = find_closure_in_expression(assignment.rhs, var_name, cursor_offset) {
                return Some(r);
            }
            find_closure_in_expression(assignment.lhs, var_name, cursor_offset)
        }
        Expression::Call(call) => match call {
            Call::Function(func_call) => {
                for arg in func_call.argument_list.arguments.iter() {
                    let arg_expr: &Expression<'_> = arg.value();
                    if let Some(r) = find_closure_in_expression(arg_expr, var_name, cursor_offset) {
                        return Some(r);
                    }
                }
                find_closure_in_expression(func_call.function, var_name, cursor_offset)
            }
            Call::Method(method_call) => {
                for arg in method_call.argument_list.arguments.iter() {
                    let arg_expr: &Expression<'_> = arg.value();
                    if let Some(r) = find_closure_in_expression(arg_expr, var_name, cursor_offset) {
                        return Some(r);
                    }
                }
                find_closure_in_expression(method_call.object, var_name, cursor_offset)
            }
            Call::StaticMethod(static_call) => {
                for arg in static_call.argument_list.arguments.iter() {
                    let arg_expr: &Expression<'_> = arg.value();
                    if let Some(r) = find_closure_in_expression(arg_expr, var_name, cursor_offset) {
                        return Some(r);
                    }
                }
                find_closure_in_expression(static_call.class, var_name, cursor_offset)
            }
            _ => None,
        },
        Expression::Parenthesized(p) => {
            find_closure_in_expression(p.expression, var_name, cursor_offset)
        }
        Expression::Instantiation(inst) => {
            if let Some(ref args) = inst.argument_list {
                for arg in args.arguments.iter() {
                    if let Some(r) =
                        find_closure_in_expression(arg.value(), var_name, cursor_offset)
                    {
                        return Some(r);
                    }
                }
            }
            None
        }
        Expression::Array(arr) => {
            for elem in arr.elements.iter() {
                let value = match elem {
                    ArrayElement::KeyValue(kv) => kv.value,
                    ArrayElement::Value(v) => v.value,
                    _ => continue,
                };
                if let Some(r) = find_closure_in_expression(value, var_name, cursor_offset) {
                    return Some(r);
                }
            }
            None
        }

        _ => None,
    }
}

/// Search parameters for a matching variable definition.
fn find_in_params(
    params: &FunctionLikeParameterList<'_>,
    var_name: &str,
    cursor_offset: u32,
) -> VarDefSearchResult {
    for param in params.parameters.iter() {
        let pname = param.variable.name.to_string();
        if pname == var_name {
            let var_start = param.variable.span.start.offset;
            let var_end = param.variable.span.end.offset;

            // Check if cursor is on this parameter's variable.
            if cursor_offset >= var_start && cursor_offset < var_end {
                return VarDefSearchResult::AtDefinition;
            }

            // Otherwise, this parameter is a definition site.
            return VarDefSearchResult::FoundAt {
                offset: var_start,
                end_offset: var_end,
            };
        }
    }
    VarDefSearchResult::NotFound
}

/// Walk a flat list of statements, collecting variable definition sites
/// that occur before the cursor.  Returns the most recent one, or
/// `AtDefinition` if the cursor is sitting on a definition.
fn find_def_in_statement_list(
    stmts: &[&Statement<'_>],
    var_name: &str,
    cursor_offset: u32,
    initial: Option<DefSite>,
) -> VarDefSearchResult {
    let mut best: Option<DefSite> = initial;

    for &stmt in stmts {
        let stmt_span = stmt.span();

        // Skip statements that start after the cursor (but we still
        // need to check foreach/try/if that *contain* the cursor).
        let starts_before_cursor = stmt_span.start.offset < cursor_offset;
        // Also allow "starts at cursor" for AtDefinition checks.
        let starts_at_or_before_cursor = stmt_span.start.offset <= cursor_offset;

        match stmt {
            Statement::Expression(expr_stmt) => {
                if !starts_at_or_before_cursor {
                    continue;
                }
                // Check for closures/arrow functions containing the cursor.
                if cursor_offset >= stmt_span.start.offset
                    && cursor_offset <= stmt_span.end.offset
                    && let Some(result) =
                        find_closure_in_expression(expr_stmt.expression, var_name, cursor_offset)
                {
                    return result;
                }
                if let Some(result) =
                    find_def_in_expression(expr_stmt.expression, var_name, cursor_offset)
                {
                    match result {
                        ExprDefResult::AtDefinition => return VarDefSearchResult::AtDefinition,
                        ExprDefResult::Found(site) => best = Some(site),
                    }
                }
            }

            Statement::Foreach(foreach) => {
                // Check the foreach key/value variables as definition sites.
                if let Some(result) = check_foreach_def(foreach, var_name, cursor_offset) {
                    match result {
                        ExprDefResult::AtDefinition => return VarDefSearchResult::AtDefinition,
                        ExprDefResult::Found(site) => best = Some(site),
                    }
                }

                // Recurse into body if cursor is inside.
                let body_span = foreach.body.span();
                if cursor_offset >= body_span.start.offset && cursor_offset <= body_span.end.offset
                {
                    let body_stmts: Vec<&Statement> = foreach.body.statements().iter().collect();
                    let result =
                        find_def_in_statement_list(&body_stmts, var_name, cursor_offset, best);
                    return result;
                }
            }

            Statement::Try(try_stmt) => {
                // Walk try block.
                let try_stmts: Vec<&Statement> = try_stmt.block.statements.iter().collect();
                let try_span = try_stmt.block.span();
                if cursor_offset >= try_span.start.offset && cursor_offset <= try_span.end.offset {
                    return find_def_in_statement_list(&try_stmts, var_name, cursor_offset, best);
                }

                // Walk catch clauses.
                for catch in try_stmt.catch_clauses.iter() {
                    // Check if catch variable matches.
                    if let Some(ref var) = catch.variable
                        && var.name == var_name
                    {
                        let var_start = var.span.start.offset;
                        let var_end = var.span.end.offset;
                        if cursor_offset >= var_start && cursor_offset < var_end {
                            return VarDefSearchResult::AtDefinition;
                        }
                        if var_start < cursor_offset {
                            best = Some(DefSite {
                                offset: var_start,
                                end_offset: var_end,
                            });
                        }
                    }

                    let catch_span = catch.block.span();
                    if cursor_offset >= catch_span.start.offset
                        && cursor_offset <= catch_span.end.offset
                    {
                        let catch_stmts: Vec<&Statement> = catch.block.statements.iter().collect();
                        return find_def_in_statement_list(
                            &catch_stmts,
                            var_name,
                            cursor_offset,
                            best,
                        );
                    }
                }

                // Walk finally clause.
                if let Some(ref finally) = try_stmt.finally_clause {
                    let finally_span = finally.block.span();
                    if cursor_offset >= finally_span.start.offset
                        && cursor_offset <= finally_span.end.offset
                    {
                        let finally_stmts: Vec<&Statement> =
                            finally.block.statements.iter().collect();
                        return find_def_in_statement_list(
                            &finally_stmts,
                            var_name,
                            cursor_offset,
                            best,
                        );
                    }
                }
            }

            Statement::If(if_stmt) => {
                // Walk all branches of the if statement.
                for inner in if_stmt.body.statements() {
                    let inner_span = inner.span();
                    if cursor_offset >= inner_span.start.offset
                        && cursor_offset <= inner_span.end.offset
                    {
                        let inner_stmts = vec![inner];
                        return find_def_in_statement_list(
                            &inner_stmts,
                            var_name,
                            cursor_offset,
                            best,
                        );
                    }
                    if starts_before_cursor && inner_span.end.offset < cursor_offset {
                        let inner_stmts = vec![inner];
                        let result =
                            find_def_in_statement_list(&inner_stmts, var_name, cursor_offset, best);
                        if let VarDefSearchResult::FoundAt { offset, end_offset } = result {
                            best = Some(DefSite { offset, end_offset });
                        }
                    }
                }
            }

            Statement::While(while_stmt) => {
                let body_span = while_stmt.body.span();
                if cursor_offset >= body_span.start.offset && cursor_offset <= body_span.end.offset
                {
                    let body_stmts: Vec<&Statement> = while_stmt.body.statements().iter().collect();
                    return find_def_in_statement_list(&body_stmts, var_name, cursor_offset, best);
                }
            }

            Statement::DoWhile(do_while) => {
                let inner_span = do_while.statement.span();
                if cursor_offset >= inner_span.start.offset
                    && cursor_offset <= inner_span.end.offset
                {
                    let inner_stmts = vec![do_while.statement];
                    return find_def_in_statement_list(&inner_stmts, var_name, cursor_offset, best);
                }
            }

            Statement::For(for_stmt) => {
                // Check initializations for assignments.
                if starts_at_or_before_cursor {
                    for init_expr in for_stmt.initializations.iter() {
                        if let Some(result) =
                            find_def_in_expression(init_expr, var_name, cursor_offset)
                        {
                            match result {
                                ExprDefResult::AtDefinition => {
                                    return VarDefSearchResult::AtDefinition;
                                }
                                ExprDefResult::Found(site) => best = Some(site),
                            }
                        }
                    }
                }
                let body_span = for_stmt.body.span();
                if cursor_offset >= body_span.start.offset && cursor_offset <= body_span.end.offset
                {
                    let body_stmts: Vec<&Statement> = for_stmt.body.statements().iter().collect();
                    return find_def_in_statement_list(&body_stmts, var_name, cursor_offset, best);
                }
            }

            Statement::Switch(switch_stmt) => {
                for case in switch_stmt.body.cases() {
                    let case_stmts: Vec<&Statement> = case.statements().iter().collect();
                    // Check if cursor is in any case.
                    for &inner in &case_stmts {
                        let inner_span: mago_span::Span = inner.span();
                        if cursor_offset >= inner_span.start.offset
                            && cursor_offset <= inner_span.end.offset
                        {
                            return find_def_in_statement_list(
                                &case_stmts,
                                var_name,
                                cursor_offset,
                                best,
                            );
                        }
                    }
                    // Scan completed cases for definitions.
                    if starts_before_cursor {
                        let result =
                            find_def_in_statement_list(&case_stmts, var_name, cursor_offset, best);
                        if let VarDefSearchResult::FoundAt { offset, end_offset } = result {
                            best = Some(DefSite { offset, end_offset });
                        }
                    }
                }
            }

            Statement::Block(block) => {
                let block_stmts: Vec<&Statement> = block.statements.iter().collect();
                let block_span = block.span();
                if cursor_offset >= block_span.start.offset
                    && cursor_offset <= block_span.end.offset
                {
                    return find_def_in_statement_list(&block_stmts, var_name, cursor_offset, best);
                }
                if starts_before_cursor {
                    let result =
                        find_def_in_statement_list(&block_stmts, var_name, cursor_offset, best);
                    if let VarDefSearchResult::FoundAt { offset, end_offset } = result {
                        best = Some(DefSite { offset, end_offset });
                    }
                }
            }

            Statement::Global(global) => {
                if !starts_at_or_before_cursor {
                    continue;
                }
                for var in global.variables.iter() {
                    if let Variable::Direct(dv) = var
                        && dv.name == var_name
                    {
                        let var_start = dv.span.start.offset;
                        let var_end = dv.span.end.offset;
                        if cursor_offset >= var_start && cursor_offset < var_end {
                            return VarDefSearchResult::AtDefinition;
                        }
                        best = Some(DefSite {
                            offset: var_start,
                            end_offset: var_end,
                        });
                    }
                }
            }

            Statement::Static(static_stmt) => {
                if !starts_at_or_before_cursor {
                    continue;
                }
                for item in static_stmt.items.iter() {
                    let dv = item.variable();
                    if dv.name == var_name {
                        let var_start = dv.span.start.offset;
                        let var_end = dv.span.end.offset;
                        if cursor_offset >= var_start && cursor_offset < var_end {
                            return VarDefSearchResult::AtDefinition;
                        }
                        best = Some(DefSite {
                            offset: var_start,
                            end_offset: var_end,
                        });
                    }
                }
            }

            Statement::Return(ret) => {
                if !starts_at_or_before_cursor {
                    continue;
                }
                if let Some(expr) = ret.value
                    && let Some(result) = find_def_in_expression(expr, var_name, cursor_offset)
                {
                    match result {
                        ExprDefResult::AtDefinition => {
                            return VarDefSearchResult::AtDefinition;
                        }
                        ExprDefResult::Found(site) => best = Some(site),
                    }
                }
            }

            _ => {}
        }
    }

    match best {
        Some(site) => VarDefSearchResult::FoundAt {
            offset: site.offset,
            end_offset: site.end_offset,
        },
        None => VarDefSearchResult::NotFound,
    }
}

/// Check if an expression contains a definition of `var_name`.
fn find_def_in_expression(
    expr: &Expression<'_>,
    var_name: &str,
    cursor_offset: u32,
) -> Option<ExprDefResult> {
    if let Expression::Assignment(assignment) = expr {
        if !assignment.operator.is_assign() {
            return None;
        }

        // ── Array destructuring: `[$a, $b] = …` / `list($a, $b) = …` ──
        match assignment.lhs {
            Expression::Array(arr) => {
                return find_var_in_destructuring_tss(&arr.elements, var_name, cursor_offset);
            }
            Expression::List(list) => {
                return find_var_in_destructuring_tss(&list.elements, var_name, cursor_offset);
            }
            _ => {}
        }

        // ── Direct variable assignment ──
        if let Expression::Variable(Variable::Direct(dv)) = assignment.lhs
            && dv.name == var_name
        {
            let var_start = dv.span.start.offset;
            let var_end = dv.span.end.offset;
            if cursor_offset >= var_start && cursor_offset < var_end {
                return Some(ExprDefResult::AtDefinition);
            }
            // When the cursor is inside the RHS of this assignment
            // (e.g. `$value = $value->value` with cursor on the RHS
            // `$value`), do NOT count this assignment as a definition
            // site.  The user wants to jump to the *original*
            // declaration (e.g. a parameter), not to the LHS of the
            // same statement.
            let rhs_span = assignment.rhs.span();
            if cursor_offset >= rhs_span.start.offset && cursor_offset <= rhs_span.end.offset {
                return None;
            }
            if var_start < cursor_offset {
                return Some(ExprDefResult::Found(DefSite {
                    offset: var_start,
                    end_offset: var_end,
                }));
            }
        }
    }

    None
}

/// Search array/list destructuring elements for our variable.
/// Search `TokenSeparatedSequence` destructuring elements (from Array/List expressions).
fn find_var_in_destructuring_tss(
    elements: &TokenSeparatedSequence<'_, ArrayElement<'_>>,
    var_name: &str,
    cursor_offset: u32,
) -> Option<ExprDefResult> {
    find_var_in_destructuring_iter(elements.iter(), var_name, cursor_offset)
}

/// Search `Sequence` destructuring elements (from foreach targets, etc.).
/// Core destructuring search implementation.
fn find_var_in_destructuring_iter<'a>(
    elements: impl Iterator<Item = &'a ArrayElement<'a>>,
    var_name: &str,
    cursor_offset: u32,
) -> Option<ExprDefResult> {
    for element in elements {
        let value = match element {
            ArrayElement::KeyValue(kv) => kv.value,
            ArrayElement::Value(v) => v.value,
            _ => continue,
        };

        // Handle nested destructuring: `[[$a, $b], $c] = …`
        match value {
            Expression::Array(arr) => {
                if let Some(r) =
                    find_var_in_destructuring_tss(&arr.elements, var_name, cursor_offset)
                {
                    return Some(r);
                }
            }
            Expression::List(list) => {
                if let Some(r) =
                    find_var_in_destructuring_tss(&list.elements, var_name, cursor_offset)
                {
                    return Some(r);
                }
            }
            Expression::Variable(Variable::Direct(dv)) if dv.name == var_name => {
                let var_start = dv.span.start.offset;
                let var_end = dv.span.end.offset;
                if cursor_offset >= var_start && cursor_offset < var_end {
                    return Some(ExprDefResult::AtDefinition);
                }
                if var_start < cursor_offset {
                    return Some(ExprDefResult::Found(DefSite {
                        offset: var_start,
                        end_offset: var_end,
                    }));
                }
            }
            _ => {}
        }
    }
    None
}

/// Check foreach key/value variables as definition sites.
fn check_foreach_def(
    foreach: &Foreach<'_>,
    var_name: &str,
    cursor_offset: u32,
) -> Option<ExprDefResult> {
    // Check value variable.
    let value_expr = foreach.target.value();
    if let Expression::Variable(Variable::Direct(dv)) = value_expr
        && dv.name == var_name
    {
        let var_start = dv.span.start.offset;
        let var_end = dv.span.end.offset;
        if cursor_offset >= var_start && cursor_offset < var_end {
            return Some(ExprDefResult::AtDefinition);
        }
        if var_start < cursor_offset {
            return Some(ExprDefResult::Found(DefSite {
                offset: var_start,
                end_offset: var_end,
            }));
        }
    }

    // Check value as destructuring: `foreach ($x as [$a, $b])`
    match value_expr {
        Expression::Array(arr) => {
            if let Some(r) = find_var_in_destructuring_tss(&arr.elements, var_name, cursor_offset) {
                return Some(r);
            }
        }
        Expression::List(list) => {
            if let Some(r) = find_var_in_destructuring_tss(&list.elements, var_name, cursor_offset)
            {
                return Some(r);
            }
        }
        _ => {}
    }

    // Check key variable.
    if let Some(key_expr) = foreach.target.key()
        && let Expression::Variable(Variable::Direct(dv)) = key_expr
        && dv.name == var_name
    {
        let var_start = dv.span.start.offset;
        let var_end = dv.span.end.offset;
        if cursor_offset >= var_start && cursor_offset < var_end {
            return Some(ExprDefResult::AtDefinition);
        }
        if var_start < cursor_offset {
            return Some(ExprDefResult::Found(DefSite {
                offset: var_start,
                end_offset: var_end,
            }));
        }
    }

    None
}
