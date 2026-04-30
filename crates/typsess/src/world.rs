use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tinymist_vfs::ImmutDict;
use tinymist_world::EntryState;
use tinymist_world::args::CompilePackageArgs;
use tinymist_world::config::CompileFontOpts;
use tinymist_world::font::{FontResolverImpl, system::SystemFontSearcher};
use tinymist_world::system::{SystemUniverseBuilder, TypstSystemWorld};
use typst::foundations::IntoValue;
use typst::syntax::VirtualPath;
use typst::utils::LazyHash;

use crate::{WorldOptions, source_error};

pub(crate) fn create_world(
    options: &WorldOptions,
) -> typst::diag::SourceResult<(TypstSystemWorld, PathBuf)> {
    let root = options
        .root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(root)
    };
    let entry = EntryState::new_rooted(
        root.clone().into(),
        Some(VirtualPath::new(Path::new("/main.typ"))),
    );
    let fonts = resolve_fonts(options).map_err(|error| source_error(error.to_string()))?;
    let package_options = CompilePackageArgs {
        package_path: options.package_path.clone(),
        package_cache_path: options.package_cache_path.clone(),
    };
    let package_registry = SystemUniverseBuilder::resolve_package(None, Some(&package_options));
    let universe = SystemUniverseBuilder::build(
        entry,
        resolve_inputs(options),
        fonts.into(),
        package_registry,
    );
    Ok((universe.snapshot(), root))
}

fn resolve_inputs(options: &WorldOptions) -> ImmutDict {
    let pairs = options
        .inputs
        .iter()
        .map(|(key, value)| (key.as_str().into(), value.as_str().into_value()));
    Arc::new(LazyHash::new(pairs.collect()))
}

fn resolve_fonts(options: &WorldOptions) -> anyhow::Result<FontResolverImpl> {
    let mut searcher = SystemFontSearcher::new();
    let embedded_fonts = if options.ignore_embedded_fonts {
        Vec::new()
    } else {
        typst_assets::fonts().map(Cow::Borrowed).collect()
    };
    searcher.resolve_opts(CompileFontOpts {
        font_paths: options.font_paths.clone(),
        no_system_fonts: options.ignore_system_fonts,
        with_embedded_fonts: embedded_fonts,
    })?;
    Ok(searcher.build())
}
