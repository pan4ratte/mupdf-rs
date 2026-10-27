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
//! **Text from embedded CIDFonts without a ToUnicode CMap.** A font subset with
//! Identity-H and no `/ToUnicode` (common in Georgian, Armenian and Indic documents made
//! by older tools) gives text extraction nothing to go by: every character comes out as
//! U+FFFD, so it can be neither copied nor searched. MuPDF leaves this as a TODO in
//! `pdf_load_to_unicode`; PDFium and Acrobat read the text off the font's own Unicode
//! cmap. `load_cid_font` now does that too when nothing else gave the font a map: each CID
//! gets the code point that the font's cmap maps to the glyph the CID is drawn with.
//!
//! **Text from symbolic Type 1 fonts whose glyph names mean nothing.** Fonts such as
//! 3B2's Advent fonts name their glyphs after their codes (`/C65`, `/C101`) and have no
//! ToUnicode, so every character comes out as U+FFFD. MuPDF means to read such a font's
//! codes as Adobe's standard encoding, but its test for a name it does not know asks
//! `fz_unicode_from_glyph_name` for 0, and that function returns U+FFFD instead, so the
//! fallback never ran (upstream's master too). The test now asks for U+FFFD. Only the
//! text changes: the glyphs were chosen before.
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
    replace_in(build_dir, "source/pdf/pdf-font.c", PDF_FONT)?;
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

const PDF_FONT: &[(&str, &str)] = &[
    (
        "static pdf_font_desc *\n\
load_cid_font(fz_context *ctx, pdf_document *doc, pdf_obj *dict, pdf_obj *encoding, pdf_obj *to_unicode, pdf_obj *fftype)\n\
{",
        "/* Folio: a ToUnicode CMap for an embedded CIDFont that has none (and none from its\n\
\x20* character collection), read off the font's own Unicode cmap, as other readers do.\n\
\x20* Without it text extraction has nothing to go by and every character comes out as\n\
\x20* U+FFFD (fonts subset with Identity-H and no ToUnicode: Georgian, Armenian and Indic\n\
\x20* documents from older tools). Each CID gets the code point the font maps to the glyph\n\
\x20* it is drawn with (pdf_font_cid_to_gid, as rendering does); where several code points\n\
\x20* map to one glyph, the lowest wins, a private use one only if there is no other. The\n\
\x20* charmap the face had is selected again afterwards. */\n\
static void\n\
pdf_synthesize_cid_to_unicode(fz_context *ctx, pdf_font_desc *fontdesc)\n\
{\n\
\tFT_Face face = fontdesc->font->ft_face;\n\
\tFT_CharMap old;\n\
\tFT_ULong ucs;\n\
\tFT_UInt gid;\n\
\tint *ucs_from_gid = NULL;\n\
\tpdf_cmap *cmap = NULL;\n\
\tint n, cid, glyph, had, pua, count = 0;\n\
\n\
\tif (!face || face->num_glyphs <= 0)\n\
\t\treturn;\n\
\n\
\tfz_var(ucs_from_gid);\n\
\tfz_var(cmap);\n\
\n\
\tfz_try(ctx)\n\
\t{\n\
\t\tucs_from_gid = fz_calloc(ctx, face->num_glyphs, sizeof *ucs_from_gid);\n\
\n\
\t\tfz_ft_lock(ctx);\n\
\t\told = face->charmap;\n\
\t\tif (FT_Select_Charmap(face, FT_ENCODING_UNICODE) == 0)\n\
\t\t{\n\
\t\t\tfor (ucs = FT_Get_First_Char(face, &gid); gid != 0; ucs = FT_Get_Next_Char(face, ucs, &gid))\n\
\t\t\t{\n\
\t\t\t\tif (gid >= (FT_UInt)face->num_glyphs || ucs < 0x20 || (ucs >= 0x7F && ucs < 0xA0) || ucs > 0x10FFFF)\n\
\t\t\t\t\tcontinue;\n\
\t\t\t\tpua = (ucs >= 0xE000 && ucs <= 0xF8FF) || ucs >= 0xF0000;\n\
\t\t\t\thad = ucs_from_gid[gid];\n\
\t\t\t\tif (had == 0 || (!pua && ((had >= 0xE000 && had <= 0xF8FF) || had >= 0xF0000)))\n\
\t\t\t\t\tucs_from_gid[gid] = (int)ucs;\n\
\t\t\t}\n\
\t\t\tif (old)\n\
\t\t\t\tFT_Set_Charmap(face, old);\n\
\t\t}\n\
\t\tfz_ft_unlock(ctx);\n\
\n\
\t\tn = fontdesc->cid_to_gid ? (int)fontdesc->cid_to_gid_len : face->num_glyphs;\n\
\t\tif (n > 0x10000)\n\
\t\t\tn = 0x10000;\n\
\t\tcmap = pdf_new_cmap(ctx);\n\
\t\tpdf_add_codespace(ctx, cmap, 0, 0xFFFF, 2);\n\
\t\tfor (cid = 0; cid < n; cid++)\n\
\t\t{\n\
\t\t\tglyph = pdf_font_cid_to_gid(ctx, fontdesc, cid);\n\
\t\t\tif (glyph > 0 && glyph < face->num_glyphs && ucs_from_gid[glyph])\n\
\t\t\t{\n\
\t\t\t\tpdf_map_range_to_range(ctx, cmap, cid, cid, ucs_from_gid[glyph]);\n\
\t\t\t\tcount++;\n\
\t\t\t}\n\
\t\t}\n\
\t\tif (count > 0)\n\
\t\t{\n\
\t\t\tpdf_sort_cmap(ctx, cmap);\n\
\t\t\tfontdesc->to_unicode = cmap;\n\
\t\t\tcmap = NULL;\n\
\t\t\tfontdesc->size += pdf_cmap_size(ctx, fontdesc->to_unicode);\n\
\t\t}\n\
\t}\n\
\tfz_always(ctx)\n\
\t{\n\
\t\tfz_free(ctx, ucs_from_gid);\n\
\t\tif (cmap)\n\
\t\t\tpdf_drop_cmap(ctx, cmap);\n\
\t}\n\
\tfz_catch(ctx)\n\
\t{\n\
\t\tfz_rethrow_if(ctx, FZ_ERROR_SYSTEM);\n\
\t\tfz_report_error(ctx);\n\
\t\tfz_warn(ctx, \"cannot read a ToUnicode map from the font's cmap\");\n\
\t}\n\
}\n\
\n\
static pdf_font_desc *\n\
load_cid_font(fz_context *ctx, pdf_document *doc, pdf_obj *dict, pdf_obj *encoding, pdf_obj *to_unicode, pdf_obj *fftype)\n\
{",
    ),
    (
        "\t\tpdf_load_to_unicode(ctx, doc, fontdesc, NULL, collection, to_unicode);\n",
        "\t\tpdf_load_to_unicode(ctx, doc, fontdesc, NULL, collection, to_unicode);\n\
\t\tif (!fontdesc->to_unicode && !fontdesc->font->flags.ft_substitute)\n\
\t\t\tpdf_synthesize_cid_to_unicode(ctx, fontdesc);\n",
    ),
    (
        "\t\t\t\tif (etable[i] && estrings[i] && !fz_unicode_from_glyph_name(estrings[i]))\n",
        "\t\t\t\tif (etable[i] && estrings[i] && fz_unicode_from_glyph_name(estrings[i]) == FZ_REPLACEMENT_CHARACTER) /* Folio: it never returns 0 */\n",
    ),
];
