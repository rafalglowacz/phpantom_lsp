/// Right-hand-side expression resolution for variable assignments.
///
/// This module resolves the type of the right-hand side of an assignment
/// (`$var = <expr>`) to zero or more `ClassInfo` values.  It handles:
///
///   - `new ClassName(…)` → the instantiated class
///   - Array access: `$arr[0]`, `$arr[$key]` → generic element type
///   - Function calls: `someFunc()` → return type
///   - Method calls: `$this->method()`, `$obj->method()` → return type
///   - Static calls: `ClassName::method()` → return type
///   - Property access: `$this->prop`, `$obj->prop` → property type
///   - Match expressions: union of all arm types
///   - Ternary / null-coalescing: union of both branches
///   - Clone: `clone $expr` → preserves the cloned expression's type
///
/// The entry point is [`resolve_rhs_expression`](Backend::resolve_rhs_expression),
/// which dispatches to specialised helpers based on the AST node kind.
/// The only caller is
/// [`check_expression_for_assignment`](Backend::check_expression_for_assignment)
/// in `variable_resolution.rs`.
use std::collections::HashMap;

use mago_span::HasSpan;
use mago_syntax::ast::*;

use crate::Backend;
use crate::docblock;
use crate::types::ClassInfo;

use super::resolution::build_var_resolver_from_ctx;
use crate::completion::call_resolution::MethodReturnCtx;
use crate::completion::conditional_resolution::resolve_conditional_with_args;
use crate::completion::resolver::VarResolutionCtx;

impl Backend {
    /// Resolve a right-hand-side expression to zero or more `ClassInfo`
    /// values.
    ///
    /// This is the single place where an arbitrary PHP expression is
    /// resolved to class types.  It handles:
    ///
    ///   - `new ClassName(…)` → the instantiated class
    ///   - Array access: `$arr[0]`, `$arr[$key]` → generic element type
    ///   - Function calls: `someFunc()` → return type
    ///   - Method calls: `$this->method()`, `$obj->method()` → return type
    ///   - Static calls: `ClassName::method()` → return type
    ///   - Property access: `$this->prop`, `$obj->prop` → property type
    ///   - Match expressions: union of all arm types
    ///   - Ternary / null-coalescing: union of both branches
    ///   - Clone: `clone $expr` → preserves the cloned expression's type
    ///
    /// Used by `check_expression_for_assignment` (for `$var = <expr>`)
    /// and recursively by multi-branch constructs (match, ternary, `??`).
    pub(in crate::completion) fn resolve_rhs_expression<'b>(
        expr: &'b Expression<'b>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        match expr {
            Expression::Instantiation(inst) => Self::resolve_rhs_instantiation(inst, ctx),
            Expression::ArrayAccess(array_access) => {
                Self::resolve_rhs_array_access(array_access, expr, ctx)
            }
            Expression::Call(call) => Self::resolve_rhs_call(call, expr, ctx),
            Expression::Access(access) => Self::resolve_rhs_property_access(access, ctx),
            Expression::Parenthesized(p) => Self::resolve_rhs_expression(p.expression, ctx),
            Expression::Match(match_expr) => {
                let mut combined = Vec::new();
                for arm in match_expr.arms.iter() {
                    let arm_results = Self::resolve_rhs_expression(arm.expression(), ctx);
                    ClassInfo::extend_unique(&mut combined, arm_results);
                }
                combined
            }
            Expression::Conditional(cond_expr) => {
                let mut combined = Vec::new();
                let then_expr = cond_expr.then.unwrap_or(cond_expr.condition);
                ClassInfo::extend_unique(
                    &mut combined,
                    Self::resolve_rhs_expression(then_expr, ctx),
                );
                ClassInfo::extend_unique(
                    &mut combined,
                    Self::resolve_rhs_expression(cond_expr.r#else, ctx),
                );
                combined
            }
            Expression::Binary(binary) if binary.operator.is_null_coalesce() => {
                let mut combined = Vec::new();
                ClassInfo::extend_unique(
                    &mut combined,
                    Self::resolve_rhs_expression(binary.lhs, ctx),
                );
                ClassInfo::extend_unique(
                    &mut combined,
                    Self::resolve_rhs_expression(binary.rhs, ctx),
                );
                combined
            }
            Expression::Clone(clone_expr) => Self::resolve_rhs_clone(clone_expr, ctx),
            Expression::PartialApplication(_)
            | Expression::Closure(_)
            | Expression::ArrowFunction(_) => {
                // First-class callable syntax (`strlen(...)`),
                // closure literals (`function() { … }`), and
                // arrow functions (`fn() => …`) all produce a
                // `Closure` instance at runtime.
                // Use the fully-qualified name so that resolution
                // succeeds even inside a namespace block (unqualified
                // class names are prefixed with the current namespace
                // and do NOT fall back to the global scope in PHP).
                crate::completion::type_resolution::type_hint_to_classes(
                    "\\Closure",
                    &ctx.current_class.name,
                    ctx.all_classes,
                    ctx.class_loader,
                )
            }
            // ── Generator yield-assignment: `$var = yield $expr` ──
            // The value of a yield expression is the TSend type from
            // the enclosing function's `@return Generator<K, V, TSend, R>`.
            Expression::Yield(_) => {
                if let Some(ref ret_type) = ctx.enclosing_return_type
                    && let Some(send_type) = crate::docblock::extract_generator_send_type(ret_type)
                {
                    return crate::completion::type_resolution::type_hint_to_classes(
                        &send_type,
                        &ctx.current_class.name,
                        ctx.all_classes,
                        ctx.class_loader,
                    );
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// Resolve `new ClassName(…)` to the instantiated class.
    fn resolve_rhs_instantiation(
        inst: &Instantiation<'_>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        let class_name = match inst.class {
            Expression::Self_(_) => Some("self"),
            Expression::Static(_) => Some("static"),
            Expression::Identifier(ident) => Some(ident.value()),
            _ => None,
        };
        if let Some(name) = class_name {
            return crate::completion::type_resolution::type_hint_to_classes(
                name,
                &ctx.current_class.name,
                ctx.all_classes,
                ctx.class_loader,
            );
        }
        vec![]
    }

    /// Resolve `$arr[0]` / `$arr[$key]` by extracting the generic element
    /// type from the base array's annotation or assignment.
    fn resolve_rhs_array_access<'b>(
        array_access: &ArrayAccess<'b>,
        expr: &'b Expression<'b>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        if let Expression::Variable(Variable::Direct(base_dv)) = array_access.array {
            let base_var = base_dv.name.to_string();
            let access_offset = expr.span().start.offset as usize;

            // Strategy 1: docblock annotation (`@var`, `@param`).
            if let Some(raw_type) =
                docblock::find_iterable_raw_type_in_source(ctx.content, access_offset, &base_var)
                && let Some(element_type) = docblock::types::extract_generic_value_type(&raw_type)
            {
                return crate::completion::type_resolution::type_hint_to_classes(
                    &element_type,
                    &ctx.current_class.name,
                    ctx.all_classes,
                    ctx.class_loader,
                );
            }

            // Strategy 2: resolve the base variable's type via AST-based
            // assignment scanning and extract the iterable element type.
            // This handles cases like `$attrs = $ref->getAttributes();`
            // where there is no explicit `@var` annotation but the method
            // return type is `ReflectionAttribute[]`.
            let current_class = Some(ctx.current_class);
            if let Some(raw_type) = super::raw_type_inference::resolve_variable_assignment_raw_type(
                &base_var,
                ctx.content,
                access_offset as u32,
                current_class,
                ctx.all_classes,
                ctx.class_loader,
                ctx.function_loader,
            ) && let Some(element_type) = docblock::types::extract_generic_value_type(&raw_type)
            {
                return crate::completion::type_resolution::type_hint_to_classes(
                    &element_type,
                    &ctx.current_class.name,
                    ctx.all_classes,
                    ctx.class_loader,
                );
            }
        }
        vec![]
    }

    /// Resolve function, method, and static method calls to their return
    /// types.
    fn resolve_rhs_call<'b>(
        call: &'b Call<'b>,
        expr: &'b Expression<'b>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        match call {
            Call::Function(func_call) => Self::resolve_rhs_function_call(func_call, expr, ctx),
            Call::Method(method_call) => Self::resolve_rhs_method_call(method_call, expr, ctx),
            Call::StaticMethod(static_call) => Self::resolve_rhs_static_call(static_call, ctx),
            _ => vec![],
        }
    }

    /// Resolve a plain function call: `someFunc()`, array functions, variable
    /// invocations (`$fn()`), and conditional return types.
    fn resolve_rhs_function_call<'b>(
        func_call: &'b FunctionCall<'b>,
        expr: &'b Expression<'b>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        let current_class_name: &str = &ctx.current_class.name;
        let all_classes = ctx.all_classes;
        let content = ctx.content;
        let class_loader = ctx.class_loader;
        let function_loader = ctx.function_loader;

        let func_name = match func_call.function {
            Expression::Identifier(ident) => Some(ident.value().to_string()),
            _ => None,
        };

        // ── Known array functions ────────────────────────
        // For element-extracting functions (array_pop, etc.)
        // resolve to the element ClassInfo directly.
        if let Some(ref name) = func_name
            && let Some(element_type) = super::raw_type_inference::resolve_array_func_element_type(
                name,
                &func_call.argument_list,
                ctx,
            )
        {
            let resolved = crate::completion::type_resolution::type_hint_to_classes(
                &element_type,
                current_class_name,
                all_classes,
                class_loader,
            );
            if !resolved.is_empty() {
                return resolved;
            }
        }

        if let Some(name) = func_name
            && let Some(fl) = function_loader
            && let Some(func_info) = fl(&name)
        {
            // Try conditional return type first
            if let Some(ref cond) = func_info.conditional_return {
                let var_resolver = build_var_resolver_from_ctx(ctx);
                let resolved_type = resolve_conditional_with_args(
                    cond,
                    &func_info.parameters,
                    &func_call.argument_list,
                    Some(&var_resolver),
                );
                if let Some(ref ty) = resolved_type {
                    let resolved = crate::completion::type_resolution::type_hint_to_classes(
                        ty,
                        current_class_name,
                        all_classes,
                        class_loader,
                    );
                    if !resolved.is_empty() {
                        return resolved;
                    }
                }
            }
            if let Some(ref ret) = func_info.return_type {
                return crate::completion::type_resolution::type_hint_to_classes(
                    ret,
                    current_class_name,
                    all_classes,
                    class_loader,
                );
            }
        }

        // ── Variable invocation: $fn() ──────────────────
        // When the callee is a variable (not a named function),
        // resolve the variable's type annotation for a
        // callable/Closure return type, or look for a
        // closure/arrow-function literal in the assignment.
        if let Expression::Variable(Variable::Direct(dv)) = func_call.function {
            let var_name = dv.name.to_string();
            let offset = expr.span().start.offset as usize;

            // 1. Try docblock annotation:
            //    `@var Closure(): User $fn` or
            //    `@param callable(int): Response $fn`
            if let Some(raw_type) =
                crate::docblock::find_iterable_raw_type_in_source(content, offset, &var_name)
                && let Some(ret) = crate::docblock::extract_callable_return_type(&raw_type)
            {
                let resolved = crate::completion::type_resolution::type_hint_to_classes(
                    &ret,
                    current_class_name,
                    all_classes,
                    class_loader,
                );
                if !resolved.is_empty() {
                    return resolved;
                }
            }

            // 2. Scan for closure literal assignment and
            //    extract native return type hint.
            if let Some(ret) =
                crate::completion::source::helpers::extract_closure_return_type_from_assignment(
                    &var_name,
                    content,
                    ctx.cursor_offset,
                )
            {
                let resolved = crate::completion::type_resolution::type_hint_to_classes(
                    &ret,
                    current_class_name,
                    all_classes,
                    class_loader,
                );
                if !resolved.is_empty() {
                    return resolved;
                }
            }

            // 3. Scan backward for first-class callable assignment:
            //    `$fn = strlen(...)`, `$fn = $obj->method(...)`, or
            //    `$fn = ClassName::staticMethod(...)`.
            //    Resolve the underlying function/method's return type.
            let rctx = ctx.as_resolution_ctx();
            if let Some(ret) =
                crate::completion::source::helpers::extract_first_class_callable_return_type(
                    &var_name, &rctx,
                )
            {
                let resolved = crate::completion::type_resolution::type_hint_to_classes(
                    &ret,
                    current_class_name,
                    all_classes,
                    class_loader,
                );
                if !resolved.is_empty() {
                    return resolved;
                }
            }
        }

        vec![]
    }

    /// Resolve an instance method call: `$this->method()`, `$var->method()`,
    /// chained calls, and other object expressions via AST-based resolution.
    fn resolve_rhs_method_call<'b>(
        method_call: &'b MethodCall<'b>,
        _expr: &'b Expression<'b>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        let method_name = match &method_call.method {
            ClassLikeMemberSelector::Identifier(ident) => ident.value.to_string(),
            // Variable method name (`$obj->$method()`) — can't resolve statically.
            _ => return vec![],
        };

        // Resolve the object expression to candidate owner classes.
        let owner_classes: Vec<ClassInfo> = if let Expression::Variable(Variable::Direct(dv)) =
            method_call.object
            && dv.name == "$this"
        {
            ctx.all_classes
                .iter()
                .find(|c| c.name == ctx.current_class.name)
                .cloned()
                .into_iter()
                .collect()
        } else if let Expression::Variable(Variable::Direct(dv)) = method_call.object {
            let var = dv.name.to_string();
            crate::completion::resolver::resolve_target_classes(
                &var,
                crate::types::AccessKind::Arrow,
                &ctx.as_resolution_ctx(),
            )
        } else {
            // Handle non-variable object expressions like
            // `(new Factory())->create()`, `getService()->method()`,
            // or chained calls by recursively resolving the expression.
            Self::resolve_rhs_expression(method_call.object, ctx)
        };

        let text_args = super::raw_type_inference::extract_argument_text(
            &method_call.argument_list,
            ctx.content,
        );
        let rctx = ctx.as_resolution_ctx();
        let var_resolver = build_var_resolver_from_ctx(ctx);

        for owner in &owner_classes {
            let template_subs = if !text_args.is_empty() {
                Self::build_method_template_subs(owner, &method_name, &text_args, &rctx)
            } else {
                HashMap::new()
            };
            let mr_ctx = MethodReturnCtx {
                all_classes: ctx.all_classes,
                class_loader: ctx.class_loader,
                template_subs: &template_subs,
                var_resolver: Some(&var_resolver),
                cache: ctx.resolved_class_cache,
            };
            let results = Self::resolve_method_return_types_with_args(
                owner,
                &method_name,
                &text_args,
                &mr_ctx,
            );
            if !results.is_empty() {
                return results;
            }
        }
        vec![]
    }

    /// Resolve a static method call: `ClassName::method()`, `self::method()`,
    /// `static::method()`.
    fn resolve_rhs_static_call(
        static_call: &StaticMethodCall<'_>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        let current_class_name: &str = &ctx.current_class.name;

        let class_name = match static_call.class {
            Expression::Self_(_) => Some(current_class_name.to_string()),
            Expression::Static(_) => Some(current_class_name.to_string()),
            Expression::Identifier(ident) => Some(ident.value().to_string()),
            _ => None,
        };
        if let Some(cls_name) = class_name
            && let ClassLikeMemberSelector::Identifier(ident) = &static_call.method
        {
            let method_name = ident.value.to_string();
            let owner = ctx
                .all_classes
                .iter()
                .find(|c| c.name == cls_name)
                .cloned()
                .or_else(|| (ctx.class_loader)(&cls_name));
            if let Some(ref owner) = owner {
                let text_args = super::raw_type_inference::extract_argument_text(
                    &static_call.argument_list,
                    ctx.content,
                );
                let rctx = ctx.as_resolution_ctx();
                let template_subs = if !text_args.is_empty() {
                    Self::build_method_template_subs(owner, &method_name, &text_args, &rctx)
                } else {
                    HashMap::new()
                };
                let var_resolver = build_var_resolver_from_ctx(ctx);
                let mr_ctx = MethodReturnCtx {
                    all_classes: ctx.all_classes,
                    class_loader: ctx.class_loader,
                    template_subs: &template_subs,
                    var_resolver: Some(&var_resolver),
                    cache: ctx.resolved_class_cache,
                };
                return Self::resolve_method_return_types_with_args(
                    owner,
                    &method_name,
                    &text_args,
                    &mr_ctx,
                );
            }
        }
        vec![]
    }

    /// Resolve property access: `$this->prop`, `$obj->prop`, `$obj?->prop`.
    fn resolve_rhs_property_access(
        access: &Access<'_>,
        ctx: &VarResolutionCtx<'_>,
    ) -> Vec<ClassInfo> {
        let current_class_name: &str = &ctx.current_class.name;
        let all_classes = ctx.all_classes;
        let class_loader = ctx.class_loader;

        let (object_expr, prop_selector) = match access {
            Access::Property(pa) => (Some(pa.object), Some(&pa.property)),
            Access::NullSafeProperty(pa) => (Some(pa.object), Some(&pa.property)),
            _ => (None, None),
        };
        if let Some(obj) = object_expr
            && let Some(sel) = prop_selector
        {
            let prop_name = match sel {
                ClassLikeMemberSelector::Identifier(ident) => Some(ident.value.to_string()),
                _ => None,
            };
            if let Some(prop_name) = prop_name {
                let owner_classes: Vec<ClassInfo> =
                    if let Expression::Variable(Variable::Direct(dv)) = obj
                        && dv.name == "$this"
                    {
                        all_classes
                            .iter()
                            .find(|c| c.name == current_class_name)
                            .cloned()
                            .into_iter()
                            .collect()
                    } else if let Expression::Variable(Variable::Direct(dv)) = obj {
                        let var = dv.name.to_string();
                        crate::completion::resolver::resolve_target_classes(
                            &var,
                            crate::types::AccessKind::Arrow,
                            &ctx.as_resolution_ctx(),
                        )
                    } else {
                        // Handle non-variable object expressions like
                        // `(new Canvas())->easel`, `getService()->prop`,
                        // or `SomeClass::make()->prop` by recursively
                        // resolving the expression type.
                        Self::resolve_rhs_expression(obj, ctx)
                    };

                for owner in &owner_classes {
                    let resolved = crate::completion::type_resolution::resolve_property_types(
                        &prop_name,
                        owner,
                        all_classes,
                        class_loader,
                    );
                    if !resolved.is_empty() {
                        return resolved;
                    }
                }
            }
        }
        vec![]
    }

    /// Resolve `clone $expr` — preserves the cloned expression's type.
    ///
    /// First tries resolving the inner expression structurally (handles
    /// `clone new Foo()`, `clone $this->getConfig()`, ternary, etc.).
    /// If that yields nothing, falls back to text-based resolution by
    /// extracting the source text of the cloned expression and resolving
    /// it as a subject string via `resolve_target_classes`.
    fn resolve_rhs_clone(clone_expr: &Clone<'_>, ctx: &VarResolutionCtx<'_>) -> Vec<ClassInfo> {
        let structural = Self::resolve_rhs_expression(clone_expr.object, ctx);
        if !structural.is_empty() {
            return structural;
        }
        // Fallback: extract source text of the cloned expression
        // and resolve it as a subject.  This handles cases like
        // `clone $original` where `$original`'s type was set by a
        // prior assignment or parameter type hint.
        let obj_span = clone_expr.object.span();
        let start = obj_span.start.offset as usize;
        let end = obj_span.end.offset as usize;
        if end <= ctx.content.len() {
            let obj_text = ctx.content[start..end].trim();
            if !obj_text.is_empty() {
                let rctx = ctx.as_resolution_ctx();
                return crate::completion::resolver::resolve_target_classes(
                    obj_text,
                    crate::types::AccessKind::Arrow,
                    &rctx,
                );
            }
        }
        vec![]
    }
}
