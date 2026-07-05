//! Macro expansion engine for Carp.
//!
//! Operates purely on [`SExpr`] trees: the expander walks the tree
//! recursively, registering top-level `defmacro` forms and expanding every
//! call to a known macro by evaluating its template body in a limited
//! compile-time DSL (`quote`, `list`, `cons`, `if`).

use std::collections::HashMap;

use crate::frontend::sexpr::SExpr;

// ---------------------------------------------------------------------------
// Macro definitions
// ---------------------------------------------------------------------------

/// A single macro registered during compilation.
#[derive(Debug, Clone)]
pub struct MacroDef {
    /// Formal parameter names.
    ///
    /// A parameter starting with `&` is a **rest** parameter — it binds to
    /// the list of all remaining arguments at the call site.  There may be
    /// at most one rest parameter, and it must be the last parameter.
    pub params: Vec<String>,
    /// The template body — an [`SExpr`] that will be evaluated in a limited
    /// compile-time context (see [`eval_macro_body`]) to produce the
    /// expanded form.
    pub body: Box<SExpr>,
}

/// The compile-time macro environment.
#[derive(Debug, Clone, Default)]
pub struct MacroEnv {
    macros: HashMap<String, MacroDef>,
}

impl MacroEnv {
    pub fn new() -> Self {
        MacroEnv { macros: HashMap::new() }
    }

    pub fn insert(&mut self, name: String, def: MacroDef) {
        self.macros.insert(name, def);
    }

    pub fn get(&self, name: &str) -> Option<&MacroDef> {
        self.macros.get(name)
    }
}

// ---------------------------------------------------------------------------
// Program-level entry point
// ---------------------------------------------------------------------------

/// Expand all macros in a sequence of top-level S-Expressions.
///
/// * Top-level `defmacro` forms are **registered** in the environment and
///   then **removed** from the output (they are compile-time directives,
///   not runtime code).
///
/// * Any other form is expanded recursively; macro calls are replaced by
///   their expansion, and the result is re-expanded (fixpoint) so that
///   macros expanding to code containing other macro calls work correctly.
pub fn expand_program(sexprs: Vec<SExpr>) -> Result<Vec<SExpr>, String> {
    let mut env = MacroEnv::new();
    let mut result = Vec::new();

    for form in sexprs {
        if is_defmacro(&form) {
            register_macro(&mut env, &form)?;
            continue;
        }
        result.push(expand(&form, &env)?);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// defmacro detection & registration
// ---------------------------------------------------------------------------

/// Is this form a top-level `defmacro` definition?
fn is_defmacro(sexpr: &SExpr) -> bool {
    matches!(
        sexpr,
        SExpr::List(list)
            if list.first() == Some(&SExpr::Symbol("defmacro".to_string()))
    )
}

/// Parse a `(defmacro name [params] body ...)` form and insert it into
/// the environment.
fn register_macro(env: &mut MacroEnv, form: &SExpr) -> Result<(), String> {
    let list = match form {
        SExpr::List(l) => l,
        _ => unreachable!(), // guarded by is_defmacro
    };

    if list.len() < 4 {
        return Err(format!(
            "malformed defmacro: expected at least (defmacro name [params] body), got {} forms",
            list.len() - 1
        ));
    }

    let name = match &list[1] {
        SExpr::Symbol(s) => s.clone(),
        other => {
            return Err(format!(
                "defmacro: name must be a symbol, got {other}"
            ));
        }
    };

    let params = parse_param_list(&list[2])?;

    // Multiple body forms are wrapped in an implicit `do`.
    let body = if list.len() == 4 {
        list[3].clone()
    } else {
        let do_forms: Vec<SExpr> = list[3..]
            .iter()
            .map(|item| {
                // Recursively expand each body form so any macro calls in
                // the template itself are expanded during definition time.
                // This allows `defmacro` templates to use other macros.
                // We use `expand` with the current env, which is safe
                // because defmacro registration happens linearly top-to-bottom.
                expand(item, env).unwrap_or_else(|_| item.clone())
            })
            .collect();
        SExpr::List({
            let mut forms = vec![SExpr::Symbol("do".to_string())];
            forms.extend(do_forms);
            forms
        })
    };

    env.insert(
        name,
        MacroDef {
            params,
            body: Box::new(body),
        },
    );

    Ok(())
}

/// Extract parameter names from `[x y & rest]`-style param lists.
///
/// A parameter preceded by `&` is a **rest** parameter — it binds to the
/// list of all remaining arguments at the call site.  Internally it is
/// stored with a `&` prefix (e.g. `&body`) so `apply_macro` can recognise
/// it.
fn parse_param_list(param_sexpr: &SExpr) -> Result<Vec<String>, String> {
    let items = match param_sexpr {
        SExpr::List(items) => items,
        other => {
            return Err(format!(
                "defmacro: parameter list must be a list, got {other}"
            ));
        }
    };

    let mut params: Vec<String> = Vec::new();
    let mut saw_ampersand = false;

    for item in items {
        match item {
            SExpr::Symbol(s) if s == "&" => {
                saw_ampersand = true;
            }
            SExpr::Symbol(s) if saw_ampersand => {
                // Mark this param as the rest param.
                params.push(format!("&{s}"));
                saw_ampersand = false;
            }
            SExpr::Symbol(s) => {
                params.push(s.clone());
            }
            other => {
                return Err(format!(
                    "defmacro: expected symbol in parameter list, got {other}"
                ));
            }
        }
    }

    if saw_ampersand {
        return Err("defmacro: '&' without a following parameter name".to_string());
    }

    Ok(params)
}

// ---------------------------------------------------------------------------
// Recursive SExpr expander
// ---------------------------------------------------------------------------

/// Recursively expand macros in a single S-Expression.
fn expand(sexpr: &SExpr, env: &MacroEnv) -> Result<SExpr, String> {
    match sexpr {
        // Atoms never contain macro calls.
        SExpr::Symbol(_)
        | SExpr::Integer(_)
        | SExpr::Float(_)
        | SExpr::String(_)
        | SExpr::Bool(_) => Ok(sexpr.clone()),

        SExpr::List(items) => {
            // Macro call?  First element is a registered symbol.
            if let Some(SExpr::Symbol(name)) = items.first() {
                if let Some(mdef) = env.get(name) {
                    return apply_macro(name, mdef, &items[1..], env);
                }
            }

            // Not a macro call — expand each sub-form recursively.
            let expanded: Result<Vec<_>, _> =
                items.iter().map(|item| expand(item, env)).collect();
            Ok(SExpr::List(expanded?))
        }
    }
}

/// Apply a macro: bind arguments to parameters, evaluate the body template
/// in the compile-time DSL, then re-expand the result (fixpoint).
fn apply_macro(
    name: &str,
    def: &MacroDef,
    args: &[SExpr],
    env: &MacroEnv,
) -> Result<SExpr, String> {
    let mut bindings: HashMap<String, SExpr> = HashMap::new();
    let mut arg_idx = 0;
    let mut rest_param: Option<&str> = None;

    for param in &def.params {
        if let Some(rest) = param.strip_prefix('&') {
            // Rest parameter — captures all remaining args as a List.
            rest_param = Some(rest);
        } else if rest_param.is_some() {
            // This shouldn't happen since we normalise params in parse_param_list.
            return Err(format!("macro '{name}': unexpected parameter after &rest"));
        } else {
            let arg = args.get(arg_idx).ok_or_else(|| {
                format!(
                    "macro '{name}': not enough arguments (expected at least {}, got {})",
                    def.params.len(),
                    args.len()
                )
            })?;
            bindings.insert(param.clone(), arg.clone());
            arg_idx += 1;
        }
    }

    // Handle the rest parameter if present.
    if let Some(rest) = rest_param {
        let remaining: Vec<SExpr> = args[arg_idx..].to_vec();
        bindings.insert(rest.to_string(), SExpr::List(remaining));
    } else if arg_idx < args.len() {
        return Err(format!(
            "macro '{name}': too many arguments (expected {}, got {})",
            def.params.len(),
            args.len()
        ));
    }

    let expanded = eval_macro_body(&def.body, &bindings, env)?;

    // Fixpoint: re-expand the result in case it contains other macro calls.
    expand(&expanded, env)
}

// ---------------------------------------------------------------------------
// Macro body evaluator (compile-time SExpr DSL)
// ---------------------------------------------------------------------------

/// Evaluate a macro body template in the limited compile-time SExpr DSL.
///
/// Supported special forms:
///
/// | Form | Behaviour |
/// |---|---|
/// | `(quote x)` | Return `x` literally, without any evaluation. |
/// | `(list e1 e2 ..)` | Evaluate each `e_i` and collect into a `List`. |
/// | `(cons e1 e2)` | Evaluate both; prepend `e1` to `e2` (which must evaluate to a `List`). |
/// | `(if cond then else?)` | Evaluate `cond`; if truthy evaluate & return `then`, otherwise `else`. |
/// | `(car xs)` | Evaluate `xs` (must be a `List`) and return the first element. |
/// | `(cdr xs)` | Evaluate `xs` (must be a `List`) and return all but the first element. |
/// | `(concat xs ys ..)` | Evaluate each argument (all must be `List`) and concatenate them. |
/// | `(nil? x)` | Returns `Bool(true)` if `x` evaluates to an empty `List`. |
/// | symbol | Look up in parameter bindings; if not bound, treat as a literal symbol. |
/// | literal | Return as-is. |
///
/// Any other `(fn args ...)` form is treated as a future macro call: each
/// argument is evaluated, and if the resulting form is a macro call it gets
/// expanded immediately.
fn eval_macro_body(
    expr: &SExpr,
    bindings: &HashMap<String, SExpr>,
    env: &MacroEnv,
) -> Result<SExpr, String> {
    match expr {
        SExpr::Integer(_) | SExpr::Float(_) | SExpr::String(_) | SExpr::Bool(_) => {
            Ok(expr.clone())
        }

        SExpr::Symbol(s) => {
            if let Some(bound) = bindings.get(s) {
                Ok(bound.clone())
            } else {
                Ok(expr.clone())
            }
        }

        SExpr::List(items) if items.is_empty() => Ok(SExpr::List(vec![])),

        SExpr::List(items) => {
            let head = &items[0];

            // --- (quote x) -------------------------------------------------
            if let SExpr::Symbol(s) = head {
                if s == "quote" {
                    if items.len() != 2 {
                        return Err(format!(
                            "quote: expected exactly one argument, got {}",
                            items.len() - 1
                        ));
                    }
                    return Ok(items[1].clone());
                }

                // --- (list e1 e2 ..) ---------------------------------------
                if s == "list" {
                    let evaluated: Result<Vec<_>, _> = items[1..]
                        .iter()
                        .map(|item| eval_macro_body(item, bindings, env))
                        .collect();
                    return Ok(SExpr::List(evaluated?));
                }

                // --- (cons e1 e2) ------------------------------------------
                if s == "cons" {
                    if items.len() != 3 {
                        return Err(format!(
                            "cons: expected exactly two arguments, got {}",
                            items.len() - 1
                        ));
                    }
                    let car = eval_macro_body(&items[1], bindings, env)?;
                    return match eval_macro_body(&items[2], bindings, env)? {
                        SExpr::List(mut list) => {
                            list.insert(0, car);
                            Ok(SExpr::List(list))
                        }
                        other => {
                            Err(format!(
                                "cons: second argument must evaluate to a list, got {other}"
                            ))
                        }
                    };
                }

                // --- (if cond then else?) -----------------------------------
                if s == "if" {
                    if items.len() < 3 {
                        return Err(format!(
                            "if: expected at least (cond then), got {} arguments",
                            items.len() - 1
                        ));
                    }
                    let cond = eval_macro_body(&items[1], bindings, env)?;
                    if is_truthy(&cond) {
                        return eval_macro_body(&items[2], bindings, env);
                    } else if items.len() >= 4 {
                        return eval_macro_body(&items[3], bindings, env);
                    } else {
                        return Ok(SExpr::Bool(false));
                    }
                }

                // --- (car xs) ----------------------------------------------
                if s == "car" {
                    if items.len() != 2 {
                        return Err(format!(
                            "car: expected exactly one argument, got {}",
                            items.len() - 1
                        ));
                    }
                    return match eval_macro_body(&items[1], bindings, env)? {
                        SExpr::List(list) => {
                            list.first().cloned().ok_or_else(|| {
                                "car: argument is an empty list".to_string()
                            })
                        }
                        other => Err(format!(
                            "car: argument must evaluate to a list, got {other}"
                        )),
                    };
                }

                // --- (cdr xs) ----------------------------------------------
                if s == "cdr" {
                    if items.len() != 2 {
                        return Err(format!(
                            "cdr: expected exactly one argument, got {}",
                            items.len() - 1
                        ));
                    }
                    return match eval_macro_body(&items[1], bindings, env)? {
                        SExpr::List(list) => {
                            if list.is_empty() {
                                Err("cdr: argument is an empty list".to_string())
                            } else {
                                Ok(SExpr::List(list[1..].to_vec()))
                            }
                        }
                        other => Err(format!(
                            "cdr: argument must evaluate to a list, got {other}"
                        )),
                    };
                }

                // --- (concat xs ys ..) -------------------------------------
                if s == "concat" {
                    let mut acc = Vec::new();
                    for arg in &items[1..] {
                        match eval_macro_body(arg, bindings, env)? {
                            SExpr::List(list) => acc.extend(list),
                            other => {
                                return Err(format!(
                                    "concat: each argument must evaluate to a list, got {other}"
                                ));
                            }
                        }
                    }
                    return Ok(SExpr::List(acc));
                }

                // --- (nil? x) ----------------------------------------------
                if s == "nil?" {
                    if items.len() != 2 {
                        return Err(format!(
                            "nil?: expected exactly one argument, got {}",
                            items.len() - 1
                        ));
                    }
                    return match eval_macro_body(&items[1], bindings, env)? {
                        SExpr::List(list) => Ok(SExpr::Bool(list.is_empty())),
                        _ => Ok(SExpr::Bool(false)),
                    };
                }
            }

            // --- default: evaluate sub-forms and maybe expand further ----
            let evaluated: Result<Vec<_>, _> = items
                .iter()
                .map(|item| eval_macro_body(item, bindings, env))
                .collect();
            let list = evaluated?;
            if let Some(SExpr::Symbol(s)) = list.first() {
                if let Some(mdef) = env.get(s) {
                    return apply_macro(s, mdef, &list[1..], env);
                }
            }
            Ok(SExpr::List(list))
        }
    }
}

/// In the macro-evaluation context, only `#f` / `false` is falsy.
fn is_truthy(expr: &SExpr) -> bool {
    match expr {
        SExpr::Bool(false) => false,
        SExpr::Symbol(s) if s == "false" || s == "nil" => false,
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::sexpr::read_source;

    /// Read source, expand macros, return the result.
    fn expand_src(source: &str) -> Result<Vec<SExpr>, String> {
        let sexprs = read_source(source)?;
        expand_program(sexprs)
    }

    /// Read and expand a single form.
    fn one_expanded(source: &str) -> SExpr {
        let mut expanded = expand_src(source).expect("expand_src should succeed");
        assert_eq!(expanded.len(), 1, "expected exactly one form");
        expanded.remove(0)
    }

    // ------------------------------------------------------------------
    // Basic defmacro + expansion
    // ------------------------------------------------------------------

    #[test]
    fn defmacro_is_consumed_not_emitted() {
        let result = expand_src("(defmacro my-macro [x] x)").unwrap();
        assert!(result.is_empty(), "defmacro should not appear in output");
    }

    #[test]
    fn when_macro_with_rest() {
        // The scanner does not have `#t` syntax — `true` is a symbol.
        let result = expand_src(
            "(defmacro when [cond & body] (list 'if cond (cons 'do body)))
             (when true (print 1) (print 2))",
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            SExpr::List(vec![
                SExpr::Symbol("if".to_string()),
                SExpr::Symbol("true".to_string()),
                SExpr::List(vec![
                    SExpr::Symbol("do".to_string()),
                    SExpr::List(vec![
                        SExpr::Symbol("print".to_string()),
                        SExpr::Integer(1),
                    ]),
                    SExpr::List(vec![
                        SExpr::Symbol("print".to_string()),
                        SExpr::Integer(2),
                    ]),
                ]),
            ])
        );
    }

    #[test]
    fn simple_macro_substitutes_args() {
        let result = expand_src(
            "(defmacro twice [x] (list '+ x x))
             (twice 5)",
        )
        .unwrap();
        assert_eq!(
            result[0],
            SExpr::List(vec![
                SExpr::Symbol("+".to_string()),
                SExpr::Integer(5),
                SExpr::Integer(5),
            ])
        );
    }

    #[test]
    fn macro_with_quoted_symbols() {
        let result = expand_src(
            "(defmacro my-cons [a b] (list 'cons a b))
             (my-cons 1 ())",
        )
        .unwrap();
        assert_eq!(
            result[0],
            SExpr::List(vec![
                SExpr::Symbol("cons".to_string()),
                SExpr::Integer(1),
                SExpr::List(vec![]),
            ])
        );
    }

    #[test]
    fn macro_body_uses_if_to_choose_expansion() {
        let result = expand_src(
            "(defmacro choose [flag a b] (if flag (list 'quote a) (list 'quote b)))
             (choose true x y)",
        )
        .unwrap();
        // `true` is a Symbol (not a Bool), and it's truthy
        assert_eq!(
            result[0],
            SExpr::List(vec![SExpr::Symbol("quote".to_string()), SExpr::Symbol("x".to_string()),])
        );
    }

    // ------------------------------------------------------------------
    // Nested macros
    // ------------------------------------------------------------------

    #[test]
    fn macro_expanding_to_macro_call_expands_again() {
        let result = expand_src(
            "(defmacro id [x] x)
             (defmacro wrap [x] (list 'id x))
             (wrap 42)",
        )
        .unwrap();
        assert_eq!(result[0], SExpr::Integer(42));
    }

    // ------------------------------------------------------------------
    // car / cdr / concat / nil? in macro bodies
    // ------------------------------------------------------------------

    #[test]
    fn macro_uses_car_and_cdr_on_args() {
        let result = expand_src(
            "(defmacro first-and-rest [xs]
               (list 'list (car xs) (cons 'list (cdr xs))))
             (first-and-rest (1 2 3))",
        )
        .unwrap();
        // (list 1 (list 2 3))
        assert_eq!(
            result[0],
            SExpr::List(vec![
                SExpr::Symbol("list".to_string()),
                SExpr::Integer(1),
                SExpr::List(vec![
                    SExpr::Symbol("list".to_string()),
                    SExpr::Integer(2),
                    SExpr::Integer(3),
                ]),
            ])
        );
    }

    #[test]
    fn macro_uses_concat() {
        let result = expand_src(
            "(defmacro prepend-all [prefix items]
               (concat prefix items))
             (prepend-all (1 2) (3 4))",
        )
        .unwrap();
        assert_eq!(
            result[0],
            SExpr::List(vec![
                SExpr::Integer(1),
                SExpr::Integer(2),
                SExpr::Integer(3),
                SExpr::Integer(4),
            ])
        );
    }

    #[test]
    fn macro_uses_nil_to_check_empty() {
        let result = expand_src(
            "(defmacro if-empty [xs then else]
               (if (nil? xs) then else))
             (if-empty () 42 0)",
        )
        .unwrap();
        assert_eq!(result[0], SExpr::Integer(42));
    }

    // ------------------------------------------------------------------
    // Error cases
    // ------------------------------------------------------------------

    #[test]
    fn defmacro_without_body_is_error() {
        let err = expand_src("(defmacro a [x])").unwrap_err();
        assert!(err.contains("malformed defmacro"), "got: {err}");
    }

    #[test]
    fn macro_too_few_args_is_error() {
        let result = expand_src("(defmacro two [a b] (list a b)) (two 1)");
        assert!(result.is_err());
    }

    #[test]
    fn macro_too_many_args_is_error() {
        let result = expand_src("(defmacro one [a] a) (one 1 2)");
        assert!(result.is_err());
    }

    // ------------------------------------------------------------------
    // Integration: expand_program strips ALL defmacros
    // ------------------------------------------------------------------

    #[test]
    fn program_with_multiple_defmacros_and_forms() {
        let result = expand_src(
            "(defmacro a [x] (list 'identity x))
             (a 1)
             (defmacro b [y] (list 'identity y))
             (b 2)
             (a 3)",
        )
        .unwrap();
        assert_eq!(result.len(), 3);
        // All three calls expand to (identity N)
        for (i, n) in [1i64, 2, 3].into_iter().enumerate() {
            assert_eq!(
                result[i],
                SExpr::List(vec![
                    SExpr::Symbol("identity".to_string()),
                    SExpr::Integer(n),
                ])
            );
        }
    }

    #[test]
    fn symbols_not_bound_in_macro_body_stay_as_symbols() {
        let result = expand_src(
            "(defmacro identity [x] (list 'the-value x))
             (identity 99)",
        )
        .unwrap();
        assert_eq!(
            result[0],
            SExpr::List(vec![
                SExpr::Symbol("the-value".to_string()),
                SExpr::Integer(99),
            ])
        );
    }
}
