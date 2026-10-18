//! Folio's changes to MuPDF's sources, applied to the build copy (MuPDF itself is not
//! forked). See https://github.com/pan4ratte/sci-pdf, docs/decisions/0008.
//!
//! **Decoding JPEG 2000 at the resolution asked for.** When MuPDF draws an image smaller
//! than it is, it asks the decoder for a reduced image (`l2factor`, halvings), and
//! subsamples whatever reduction the decoder could not do. Every format honours this except
//! JPEG 2000, which is always decoded in full: a 33-megapixel scan drawn at 1400 pixels
//! wide costs the full decode (about 0.9 s) and is then shrunk. OpenJPEG can decode at a
//! reduced resolution itself (`opj_set_decoded_resolution_factor`), as far as the
//! codestream's resolution levels go; this asks it to.
//!
//! - Only images placed at the origin of their reference grid are reduced (almost all of
//!   them); others decode in full as before.
//! - If a reduced decode fails (a tile with fewer resolution levels than the default),
//!   the image is decoded in full, as before.
//! - No public header changes: `fz_load_jpx_reduced` is declared where it is used.
//!
//! **Decoding JPEG with libjpeg-turbo** (the `libjpeg-turbo` feature). MuPDF bundles the
//! Independent JPEG Group's libjpeg, which has no SIMD code: a 16-megapixel CMYK JPEG takes
//! about 300 ms to decode, where libjpeg-turbo takes about 90. MuPDF already builds against
//! an outside libjpeg (`SHARE_JPEG`, as Linux distributions build it with libjpeg-turbo);
//! with the feature on, it is built that way against the libjpeg-turbo decoder that
//! `mozjpeg-sys` compiles and links (MozJPEG changes only libjpeg-turbo's encoder), and its
//! own libjpeg is left out. The make build is told so through `USE_SYSTEM_LIBJPEG`
//! (make.rs); the MSBuild projects are patched here (`libjpeg_turbo_msbuild`).
//!
//! Each patch must match exactly once, so a MuPDF update that moves the code fails the
//! build here instead of silently dropping the change.

use std::fs;
use std::path::Path;

use crate::Result;

pub fn apply(build_dir: &Path) -> Result<()> {
    replace_in(build_dir, "source/fitz/load-jpx.c", LOAD_JPX)?;
    replace_in(build_dir, "source/fitz/image.c", IMAGE)?;
    replace_in(build_dir, "source/fitz/filter-dct.c", FILTER_DCT)?;
    Ok(())
}

/// The folders holding libjpeg-turbo's headers (as a path list), when the `libjpeg-turbo`
/// feature is on: `mozjpeg-sys` publishes them as the `include` of its `links = "jpeg"`.
pub fn libjpeg_turbo_include() -> Result<Option<String>> {
    if !cfg!(feature = "libjpeg-turbo") {
        return Ok(None);
    }
    let include = std::env::var("DEP_JPEG_INCLUDE")
        .map_err(|_| "Folio: the libjpeg-turbo feature is on, but mozjpeg-sys gave no DEP_JPEG_INCLUDE")?;
    Ok(Some(include))
}

/// The MSBuild side of the `libjpeg-turbo` feature: libthirdparty no longer compiles
/// MuPDF's libjpeg, and libmupdf is compiled with `SHARE_JPEG` against libjpeg-turbo's
/// headers in place of libjpeg's.
pub fn libjpeg_turbo_msbuild(build_dir: &Path) -> Result<()> {
    let Some(include) = libjpeg_turbo_include()? else {
        return Ok(());
    };
    let include: Vec<String> = std::env::split_paths(&include)
        .map(|p| p.display().to_string())
        .collect();

    let path = build_dir.join("platform/win32/libthirdparty.vcxproj");
    let source = read(&path)?;
    let before = source.lines().count();
    let kept: Vec<&str> = source
        .lines()
        .filter(|line| !line.contains(r"..\..\thirdparty\libjpeg\"))
        .collect();
    if kept.len() == before {
        Err("Folio patch for libthirdparty.vcxproj: found none of libjpeg's sources")?;
    }
    write(&path, &(kept.join("\r\n") + "\r\n"))?;

    let path = build_dir.join("platform/win32/libmupdf.vcxproj");
    let mut source = read(&path)?;
    for (old, new) in [
        (r"..\..\scripts\libjpeg;".to_owned(), format!("{};", include.join(";"))),
        (r"..\..\thirdparty\libjpeg;".to_owned(), String::new()),
        ("<PreprocessorDefinitions>".to_owned(), "<PreprocessorDefinitions>SHARE_JPEG;".to_owned()),
    ] {
        if !source.contains(&old) {
            Err(format!("Folio patch for libmupdf.vcxproj: found no {old}"))?;
        }
        source = source.replace(&old, &new);
    }
    write(&path, &source)
}

fn read(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path).map_err(|e| format!("Unable to read {}: {e}", path.display()))?)
}

fn write(path: &Path, source: &str) -> Result<()> {
    Ok(fs::write(path, source).map_err(|e| format!("Unable to write {}: {e}", path.display()))?)
}

fn replace_in(build_dir: &Path, file: &str, patches: &[(&str, &str)]) -> Result<()> {
    let path = build_dir.join(file);
    let mut source = fs::read_to_string(&path)
        .map_err(|e| format!("Unable to read {}: {e}", path.display()))?
        .replace("\r\n", "\n");
    for (old, new) in patches {
        let found = source.matches(old).count();
        if found != 1 {
            Err(format!(
                "Folio patch for {file}: expected one match, found {found}:\n{old}"
            ))?;
        }
        source = source.replacen(old, new, 1);
    }
    fs::write(&path, source).map_err(|e| format!("Unable to write {}: {e}", path.display()))?;
    Ok(())
}

const LOAD_JPX: &[(&str, &str)] = &[
    (
        "size_t size, fz_colorspace *defcs, int onlymeta)\n{",
        "size_t size, fz_colorspace *defcs, int onlymeta, int *reduce)\n{",
    ),
    (
        "\t\tfz_throw(ctx, FZ_ERROR_LIBRARY, \"Failed to read JPX header\");\n\t}\n",
        "\t\tfz_throw(ctx, FZ_ERROR_LIBRARY, \"Failed to read JPX header\");\n\t}\n\
\n\
\t/* Folio: decode at a reduced resolution when asked to (*reduce halvings), as far as\n\
\t * the codestream's resolution levels allow; *reduce becomes what was applied. */\n\
\tif (reduce)\n\
\t{\n\
\t\tint wanted = *reduce;\n\
\t\t*reduce = 0;\n\
\t\tif (!onlymeta && wanted > 0 && jpx->x0 == 0 && jpx->y0 == 0)\n\
\t\t{\n\
\t\t\topj_codestream_info_v2_t *info = opj_get_cstr_info(codec);\n\
\t\t\tOPJ_UINT32 c, levels = 0;\n\
\t\t\tif (info)\n\
\t\t\t{\n\
\t\t\t\tif (info->m_default_tile_info.tccp_info)\n\
\t\t\t\t{\n\
\t\t\t\t\tlevels = 33;\n\
\t\t\t\t\tfor (c = 0; c < info->nbcomps; c++)\n\
\t\t\t\t\t\tif (info->m_default_tile_info.tccp_info[c].numresolutions < levels)\n\
\t\t\t\t\t\t\tlevels = info->m_default_tile_info.tccp_info[c].numresolutions;\n\
\t\t\t\t}\n\
\t\t\t\topj_destroy_cstr_info(&info);\n\
\t\t\t}\n\
\t\t\tif (levels > 0 && wanted > (int)levels - 1)\n\
\t\t\t\twanted = (int)levels - 1;\n\
\t\t\tif (levels > 0 && wanted > 0 && opj_set_decoded_resolution_factor(codec, (OPJ_UINT32)wanted))\n\
\t\t\t\t*reduce = wanted;\n\
\t\t}\n\
\t}\n",
    ),
    (
        "\tw = state->width = jpx->x1 - jpx->x0;\n\th = state->height = jpx->y1 - jpx->y0;\n",
        "\tw = state->width = jpx->x1 - jpx->x0;\n\th = state->height = jpx->y1 - jpx->y0;\n\
\t/* Folio: a reduced decode is smaller (OpenJPEG keeps the full reference grid). */\n\
\tif (reduce && *reduce > 0 && w > 0 && h > 0)\n\
\t{\n\
\t\tw = (int)(((OPJ_UINT32)w + (1u << *reduce) - 1) >> *reduce);\n\
\t\th = (int)(((OPJ_UINT32)h + (1u << *reduce) - 1) >> *reduce);\n\
\t}\n",
    ),
    (
        "pix = jpx_read_image(ctx, &state, data, size, defcs, 0);",
        "pix = jpx_read_image(ctx, &state, data, size, defcs, 0, NULL);",
    ),
    (
        "jpx_read_image(ctx, &state, data, size, defcs, 1);",
        "jpx_read_image(ctx, &state, data, size, defcs, 1, NULL);",
    ),
    (
        "void\nfz_load_jpx_info(fz_context *ctx, const unsigned char *data, size_t size, int *wp, int *hp, int *xresp, int *yresp, fz_colorspace **cspacep, fz_colorspace *defcs)\n{",
        "/* Folio: like fz_load_jpx, decoded *reduce halvings smaller where the codestream\n\
\x20* allows; *reduce becomes the reduction applied (0 to *reduce). */\n\
fz_pixmap *fz_load_jpx_reduced(fz_context *ctx, const unsigned char *data, size_t size, fz_colorspace *defcs, int *reduce);\n\
\n\
fz_pixmap *\n\
fz_load_jpx_reduced(fz_context *ctx, const unsigned char *data, size_t size, fz_colorspace *defcs, int *reduce)\n\
{\n\
\tfz_jpxd state = { 0 };\n\
\tfz_pixmap *pix = NULL;\n\
\tint wanted = *reduce;\n\
\tint failed = 0;\n\
\n\
\tif (wanted <= 0)\n\
\t{\n\
\t\t*reduce = 0;\n\
\t\treturn fz_load_jpx(ctx, data, size, defcs);\n\
\t}\n\
\n\
\tfz_try(ctx)\n\
\t{\n\
\t\tfz_opj_lock(ctx);\n\
\t\tpix = jpx_read_image(ctx, &state, data, size, defcs, 0, reduce);\n\
\t}\n\
\tfz_always(ctx)\n\
\t\tfz_opj_unlock(ctx);\n\
\tfz_catch(ctx)\n\
\t{\n\
\t\tfz_rethrow_if(ctx, FZ_ERROR_SYSTEM);\n\
\t\tfz_report_error(ctx);\n\
\t\tfailed = 1;\n\
\t}\n\
\n\
\tif (failed)\n\
\t{\n\
\t\t/* A reduced decode failed: decode in full, as MuPDF always did. */\n\
\t\t*reduce = 0;\n\
\t\tpix = fz_load_jpx(ctx, data, size, defcs);\n\
\t}\n\
\treturn pix;\n\
}\n\
\n\
void\nfz_load_jpx_info(fz_context *ctx, const unsigned char *data, size_t size, int *wp, int *hp, int *xresp, int *yresp, fz_colorspace **cspacep, fz_colorspace *defcs)\n{",
    ),
    (
        "#else /* FZ_ENABLE_JPX */\n",
        "#else /* FZ_ENABLE_JPX */\n\
\n\
fz_pixmap *fz_load_jpx_reduced(fz_context *ctx, const unsigned char *data, size_t size, fz_colorspace *defcs, int *reduce);\n\
\n\
fz_pixmap *\n\
fz_load_jpx_reduced(fz_context *ctx, const unsigned char *data, size_t size, fz_colorspace *defcs, int *reduce)\n\
{\n\
\tfz_throw(ctx, FZ_ERROR_UNSUPPORTED, \"JPX support disabled\");\n\
}\n",
    ),
];

/// The JPEG filter hands its output on in 4 KB pieces; a row wider than that (any image
/// over 1,024 RGB pixels across) is decoded into a row buffer and was copied out a byte at
/// a time, 65 million times for a 16-megapixel CMYK image. Copied with memcpy instead.
const FILTER_DCT: &[(&str, &str)] = &[
    (
        "\t\t\tstate->wp = state->scanline;\n\t\t}\n\n\t\twhile (state->rp < state->wp && p < ep)\n\t\t\t*p++ = *state->rp++;\n",
        "\t\t\tstate->wp = state->scanline;\n\t\t}\n\n\t\t/* Folio: memcpy, not a byte at a time. */\n\t\tif (state->rp < state->wp && p < ep)\n\t\t{\n\t\t\tsize_t n = fz_minz(state->wp - state->rp, ep - p);\n\t\t\tmemcpy(p, state->rp, n);\n\t\t\tp += n;\n\t\t\tstate->rp += n;\n\t\t}\n",
    ),
    (
        "\t\t\t\tstate->wp = state->scanline + state->stride;\n\t\t\t}\n\n\t\t\twhile (state->rp < state->wp && p < ep)\n\t\t\t\t*p++ = *state->rp++;\n",
        "\t\t\t\tstate->wp = state->scanline + state->stride;\n\t\t\t}\n\n\t\t\t/* Folio: memcpy, not a byte at a time. */\n\t\t\tif (state->rp < state->wp && p < ep)\n\t\t\t{\n\t\t\t\tsize_t n = fz_minz(state->wp - state->rp, ep - p);\n\t\t\t\tmemcpy(p, state->rp, n);\n\t\t\t\tp += n;\n\t\t\t\tstate->rp += n;\n\t\t\t}\n",
    ),
];

const IMAGE: &[(&str, &str)] = &[
    (
        "static fz_pixmap *\ncompressed_image_get_pixmap(fz_context *ctx, fz_image *image_, fz_irect *subarea, int w, int h, int *l2factor)\n{",
        "/* Folio: load-jpx.c */\n\
fz_pixmap *fz_load_jpx_reduced(fz_context *ctx, const unsigned char *data, size_t size, fz_colorspace *defcs, int *reduce);\n\
\n\
static fz_pixmap *\ncompressed_image_get_pixmap(fz_context *ctx, fz_image *image_, fz_irect *subarea, int w, int h, int *l2factor)\n{",
    ),
    (
        "\t\ttile = fz_load_jpx(ctx, image->buffer->buffer->data, image->buffer->buffer->len, image->super.colorspace);\n",
        "\t\t{\n\
\t\t\t/* Folio: decode as reduced as asked, where the codestream allows; the caller\n\
\t\t\t * subsamples what remains in *l2factor. */\n\
\t\t\tint reduce = l2factor ? *l2factor : 0;\n\
\t\t\ttile = fz_load_jpx_reduced(ctx, image->buffer->buffer->data, image->buffer->buffer->len, image->super.colorspace, &reduce);\n\
\t\t\tif (l2factor)\n\
\t\t\t\t*l2factor -= reduce;\n\
\t\t}\n",
    ),
];
