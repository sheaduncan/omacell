//! `LET`, `LAMBDA`, and `ISOMITTED` (F-3.4 language constructs).

use std::sync::Arc;

use crate::coerce::Scalar;
use crate::error::ErrorKind;
use crate::eval::{ArgVal, EvalCtx, RuntimeValue};
use crate::formula::{Expr, ExprKind};

/// One LAMBDA parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LambdaParam {
    /// Binding name (original spelling).
    pub name: String,
}

/// A closure: parameter list, body, and captured `LET`/`LAMBDA` bindings.
#[derive(Clone, Debug)]
pub struct Lambda {
    /// Parameters in order.
    pub params: Arc<[LambdaParam]>,
    /// Body expression.
    pub body: Expr,
    /// Captured scope (name, value), innermost last.
    pub closure: Arc<[(String, RuntimeValue)]>,
}

impl PartialEq for Lambda {
    fn eq(&self, other: &Self) -> bool {
        self.params == other.params && self.body == other.body
    }
}

/// Whether `name` is a language construct owned by this module.
#[must_use]
pub fn is_language_fn(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "LET" | "LAMBDA" | "ISOMITTED"
    )
}

/// Extract a parameter name from a LET/LAMBDA argument expression.
pub fn param_of(expr: &Expr) -> Option<LambdaParam> {
    match &expr.kind {
        ExprKind::Name { name, .. } => Some(LambdaParam { name: name.clone() }),
        _ => None,
    }
}

/// Evaluate `LET(name, value, ..., calc)`.
pub fn eval_let(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    // At least name, value, calc.
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let calc = match args.last() {
        Some(Some(e)) => e,
        _ => return RuntimeValue::error(ErrorKind::Value),
    };
    let n_binds = (args.len() - 1) / 2;
    ctx.push_frame();
    for i in 0..n_binds {
        let name_e = match &args[i * 2] {
            Some(e) => e,
            None => {
                ctx.pop_frame();
                return RuntimeValue::error(ErrorKind::Value);
            }
        };
        let Some(p) = param_of(name_e) else {
            ctx.pop_frame();
            return RuntimeValue::error(ErrorKind::Value);
        };
        let val_e = match &args[i * 2 + 1] {
            Some(e) => e,
            None => {
                ctx.pop_frame();
                return RuntimeValue::error(ErrorKind::Value);
            }
        };
        let val = crate::eval::eval_expr(ctx, val_e);
        if let Some(e) = val.error_kind() {
            ctx.pop_frame();
            return RuntimeValue::error(e);
        }
        ctx.bind(p.name, val);
    }
    let out = crate::eval::eval_expr(ctx, calc);
    ctx.pop_frame();
    out
}

/// Evaluate a `LAMBDA(...)` *definition* (not a call).
pub fn eval_lambda_def(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    if args.is_empty() {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let body = match args.last() {
        Some(Some(e)) => e.clone(),
        _ => return RuntimeValue::error(ErrorKind::Value),
    };
    let mut params = Vec::with_capacity(args.len() - 1);
    for a in &args[..args.len() - 1] {
        let Some(e) = a else {
            return RuntimeValue::error(ErrorKind::Value);
        };
        let Some(p) = param_of(e) else {
            return RuntimeValue::error(ErrorKind::Value);
        };
        params.push(p);
    }
    RuntimeValue::Lambda(Arc::new(Lambda {
        params: params.into(),
        body,
        closure: ctx.snapshot_scope(),
    }))
}

/// `ISOMITTED(x)` — true when `x` is an omitted optional LAMBDA argument.
pub fn eval_isomitted(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    if args.len() != 1 {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let Some(expr) = args[0].as_ref() else {
        return RuntimeValue::Scalar(Scalar::Bool(true));
    };
    if let ExprKind::Name { name, .. } = &expr.kind
        && ctx.is_omitted(name)
    {
        return RuntimeValue::Scalar(Scalar::Bool(true));
    }
    // Evaluate: an explicit value is not omitted.
    let v = crate::eval::eval_expr(ctx, expr);
    match v {
        RuntimeValue::Scalar(Scalar::Empty) if matches!(&expr.kind, ExprKind::Name { .. }) => {
            RuntimeValue::Scalar(Scalar::Bool(ctx.is_omitted_expr(expr)))
        }
        _ => RuntimeValue::Scalar(Scalar::Bool(false)),
    }
}

/// Apply a lambda to evaluated arguments.
pub fn apply(ctx: &mut EvalCtx<'_>, lam: &Lambda, args: &[ArgVal]) -> RuntimeValue {
    // An omitted placeholder still occupies an argument position (`(1,)`).
    // Fewer or extra positions are an incorrect LAMBDA invocation in Excel.
    if args.len() != lam.params.len() {
        return RuntimeValue::error(ErrorKind::Value);
    }
    if ctx.enter_call().is_err() {
        return RuntimeValue::error(ErrorKind::Num);
    }
    ctx.push_frame();
    for (name, val) in lam.closure.iter() {
        ctx.bind(name.clone(), val.clone());
    }
    for (i, p) in lam.params.iter().enumerate() {
        if let Some(a) = args.get(i) {
            if a.omitted {
                ctx.bind_omitted(p.name.clone());
            } else {
                ctx.bind(p.name.clone(), a.value.clone());
            }
        }
    }
    let out = crate::eval::eval_expr(ctx, &lam.body);
    ctx.pop_frame();
    ctx.leave_call();
    out
}

/// Apply if `callee` is a lambda; otherwise `#VALUE!`.
pub fn apply_value(ctx: &mut EvalCtx<'_>, callee: RuntimeValue, args: &[ArgVal]) -> RuntimeValue {
    match callee {
        RuntimeValue::Lambda(l) => apply(ctx, &l, args),
        RuntimeValue::Scalar(Scalar::Error(e)) => RuntimeValue::error(e),
        _ => RuntimeValue::error(ErrorKind::Value),
    }
}
