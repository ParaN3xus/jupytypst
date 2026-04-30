use typst::diag::At;
use typst::foundations::{Content, Scope, Style, Styles, Value, ops};
use typst::syntax::{Source, ast, ast::AstNode};
use typst_eval::{Eval, Vm};

use crate::{SourceMode, source_error};

pub(crate) const CODE_WRAPPER_PREFIX: &str = "#{\n";
pub(crate) const CODE_WRAPPER_SUFFIX: &str = "\n}";

pub(crate) struct EvaluatedCell {
    pub(crate) content: Content,
    pub(crate) warnings: ecow::EcoVec<typst::diag::SourceDiagnostic>,
    pub(crate) source_map_index: usize,
}

pub(crate) struct EvaluatedSource {
    pub(crate) value: Value,
    pub(crate) scope: Scope,
    pub(crate) captured_styles: Styles,
    pub(crate) warnings: ecow::EcoVec<typst::diag::SourceDiagnostic>,
    pub(crate) source_map_index: usize,
}

pub(crate) fn eval_source_capture(
    vm: &mut Vm,
    root: &typst::syntax::SyntaxNode,
    source_mode: SourceMode,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Value> {
    match source_mode {
        SourceMode::Code => {
            let code = wrapped_code_body(root)?;
            eval_code_capture(vm, &mut code.exprs(), filter_styles, captured_styles)
        }
        SourceMode::Markup => {
            let markup = root
                .cast::<ast::Markup>()
                .ok_or_else(|| source_error("failed to parse Typst markup"))?;
            Ok(Value::Content(eval_markup_capture(
                vm,
                &mut markup.exprs(),
                filter_styles,
                captured_styles,
            )?))
        }
    }
}

pub(crate) fn parsed_source(
    file_id: typst::syntax::FileId,
    source: &str,
    mode: SourceMode,
) -> Source {
    let text = match mode {
        SourceMode::Code => format!("{CODE_WRAPPER_PREFIX}{source}{CODE_WRAPPER_SUFFIX}"),
        SourceMode::Markup => source.to_string(),
    };
    Source::new(file_id, text)
}

pub(crate) fn span_offset(mode: SourceMode) -> usize {
    match mode {
        SourceMode::Code => CODE_WRAPPER_PREFIX.len(),
        SourceMode::Markup => 0,
    }
}

fn eval_code_capture<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Value> {
    let flow = vm.flow.take();
    let mut output = Value::None;

    while let Some(expr) = exprs.next() {
        let value = match expr {
            ast::Expr::SetRule(set) => {
                let styles = set.eval(vm)?;
                captured_styles.apply(filter_styles(styles.clone()));
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_code_capture(vm, exprs, filter_styles, captured_styles)?.display();
                Value::Content(tail.styled_with_map(styles))
            }
            ast::Expr::ShowRule(show) => {
                let recipe = show.eval(vm)?;
                captured_styles.apply(Style::from(recipe.clone()).into());
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_code_capture(vm, exprs, filter_styles, captured_styles)?.display();
                Value::Content(tail.styled_with_recipe(&mut vm.engine, vm.context, recipe)?)
            }
            ast::Expr::CodeBlock(block) => {
                eval_code_block_capture(vm, block, filter_styles, captured_styles)?
            }
            _ => {
                let span = expr.span();
                let value = expr.eval(vm)?;
                output = ops::join(output, value).at(span)?;

                if vm.flow.is_some() {
                    break;
                }
                continue;
            }
        };

        output = ops::join(output, value).at(expr.span())?;

        if vm.flow.is_some() {
            break;
        }
    }

    if flow.is_some() {
        vm.flow = flow;
    }

    Ok(output)
}

fn eval_markup_capture<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Content> {
    let flow = vm.flow.take();
    let mut output = Vec::new();

    while let Some(expr) = exprs.next() {
        match expr {
            ast::Expr::SetRule(set) => {
                let styles = set.eval(vm)?;
                captured_styles.apply(filter_styles(styles.clone()));
                if vm.flow.is_some() {
                    break;
                }
                output.push(
                    eval_markup_capture(vm, exprs, filter_styles, captured_styles)?
                        .styled_with_map(styles),
                );
            }
            ast::Expr::ShowRule(show) => {
                let recipe = show.eval(vm)?;
                captured_styles.apply(Style::from(recipe.clone()).into());
                if vm.flow.is_some() {
                    break;
                }
                let tail = eval_markup_capture(vm, exprs, filter_styles, captured_styles)?;
                output.push(tail.styled_with_recipe(&mut vm.engine, vm.context, recipe)?);
            }
            expr => {
                let value = expr.eval(vm)?;
                if !matches!(value, Value::Label(_)) {
                    output.push(value.display().spanned(expr.span()));
                }
            }
        }

        if vm.flow.is_some() {
            break;
        }
    }

    if flow.is_some() {
        vm.flow = flow;
    }

    Ok(Content::sequence(output))
}

fn eval_code_block_capture(
    vm: &mut Vm,
    block: ast::CodeBlock,
    filter_styles: &dyn Fn(Styles) -> Styles,
    captured_styles: &mut Styles,
) -> typst::diag::SourceResult<Value> {
    vm.scopes.enter();
    let output = eval_code_capture(
        vm,
        &mut block.body().exprs(),
        filter_styles,
        captured_styles,
    );
    vm.scopes.exit();
    output
}

fn wrapped_code_body(root: &typst::syntax::SyntaxNode) -> typst::diag::SourceResult<ast::Code<'_>> {
    let markup = root
        .cast::<ast::Markup>()
        .ok_or_else(|| source_error("failed to parse wrapped Typst code"))?;
    markup
        .exprs()
        .find_map(|expr| match expr {
            ast::Expr::CodeBlock(block) => Some(block.body()),
            _ => None,
        })
        .ok_or_else(|| source_error("failed to find wrapped Typst code body"))
}
