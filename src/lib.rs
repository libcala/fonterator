// lib.rs -- Fonterator
// Copyright (c) 2018  Jeron Lau <jeron.lau@plopgrizzly.com>
// Copyright (c) 2016  Dylan Ede
// Dual-Licensed under the MIT LICENSE and the APACHE LICENSE

//! Fonterator is a pure Rust alternative to libraries like FreeType based on
//! RustType.
//!
//! The current capabilities of Fonterator:
//!
//! * Reading TrueType formatted fonts and font collections. This includes
//!   `*.ttf` as well as a subset of `*.otf` font files.
//! * Retrieving glyph shapes and commonly used properties for a font and its
//!   glyphs.
//! * Laying out glyphs horizontally using horizontal and vertical metrics, and
//!   glyph-pair-specific kerning.
//!
//! Notable things that Fonterator does not support *yet*:
//!
//! * OpenType formatted fonts that are not just TrueType fonts (OpenType is a
//!   superset of TrueType). Notably there is no support yet for cubic Bezier
//!   curves used in glyphs.
//! * Ligatures of any kind (‽, etc.).
//! * Some less common TrueType sub-formats.
//! * Right-to-left and vertical text layout.
//!
//! # Getting Started
//!
//! Add the following to your Cargo.toml:
//!
//! ```toml
//! [dependencies]
//! fonterator = "0.1.0"
//! ```
//!
//! To hit the ground running with Fonterator, look at the `image.rs` example
//! supplied with the crate. It demonstrates loading a font file, rasterising an
//! arbitrary string, and saving as an SVG. If you prefer to
//! just look at the documentation, the entry point for loading fonts is
//! `FontCollection`, from which you can access individual fonts, then their
//! glyphs.
//!
//! # Unicode terminology
//!
//! This crate uses terminology for computerised typography as specified by the
//! Unicode standard. If you are not sure of the differences between a code
//! point, a character, and a glyph, you may want to check the [official Unicode
//! glossary](http://unicode.org/glossary/), or alternatively, here's my take on
//! it from a practical perspective:
//!
//! * A character is what you would conventionally call a single symbol,
//!   independent of its appearance or representation in a particular font.
//!   Examples include `a`, `A`, `ä`, `å`, `1`, `*`, `Ω`, etc.
//! * A Unicode code point is the particular number that the Unicode standard
//!   associates with a particular character. Note however that code points also
//!   exist for things not conventionally thought of as characters by
//!   themselves, but can be combined to form characters, such as diacritics
//!   like accents. These "characters" are known in Unicode as "combining
//!   characters". E.g., a diaeresis (`¨`) has the code point U+0308. If this
//!   code point follows the code point U+0055 (the letter `u`), this sequence
//!   represents the character `ü`. Note that there is also a single codepoint
//!   for `ü`, U+00FC. This means that what visually looks like the same string
//!   can have multiple different Unicode representations. Some fonts will have
//!   glyphs (see below) for one sequence of codepoints, but not another that
//!   has the same meaning. To deal with this problem it is recommended to use
//!   Unicode normalisation, as provided by, for example, the
//!   [unicode-normalization](http://crates.io/crates/unicode-normalization)
//!   crate, to convert to code point sequences that work with the font in
//!   question. Typically a font is more likely to support a single code point
//!   vs. a sequence with the same meaning, so the best normalisation to use is
//!   "canonical recomposition", known as NFC in the normalisation crate.
//! * A glyph is a particular font's shape to draw the character for a
//!   particular Unicode code point. This will have its own identifying number
//!   unique to the font, its ID.

extern crate ordered_float;
extern crate stb_truetype;
extern crate unicode_normalization;

use unicode_normalization::UnicodeNormalization;

use stb_truetype as tt;
use std::fmt;
use std::sync::Arc;

/// A 2D vector
#[derive(Copy, Clone)]
pub struct Vec2(pub f32, pub f32);

/// An iterator over `PathOp`.
pub struct Path(Vec<PathOp>);

impl IntoIterator for Path {
	type Item = PathOp;
	type IntoIter = ::std::vec::IntoIter<PathOp>;

	fn into_iter(self) -> Self::IntoIter {
		self.0.into_iter()
	}
}

/// An operation that builds a path.
pub enum PathOp {
	/// Move somewhere else `x, y`.
	MoveTo(f32, f32),
	/// Next point in edge `x, y`
	LineTo(f32, f32),
	/// Quadratic curve `x, y, cx, cy`
	QuadTo(f32, f32, f32, f32),
	/// Close the path with a line
	LineClose,
	/// Close the path with a quadratic curve `cx, cy`
	QuadClose(f32, f32),
}

/// A collection of fonts read straight from a font file's data. The data in the
/// collection is not validated. This structure may or may not own the font
/// data.
#[derive(Clone, Debug)]
pub struct FontCollection<'a>(SharedBytes<'a>);
/// A single font. This may or may not own the font data.
#[derive(Clone)]
pub struct Font<'a> {
	info: tt::FontInfo<SharedBytes<'a>>,
}

/// `SharedBytes` handles the lifetime of font data used in Fonterator. The data
/// is either a shared reference to externally owned data, or managed by
/// reference counting. `SharedBytes` can be conveniently used with `From` and
/// `Into`, and dereferences to the contained bytes.
#[derive(Clone, Debug)]
pub enum SharedBytes<'a> {
	ByRef(&'a [u8]),
	ByArc(Arc<[u8]>),
}

impl<'a> ::std::ops::Deref for SharedBytes<'a> {
	type Target = [u8];
	fn deref(&self) -> &[u8] {
		match *self {
			SharedBytes::ByRef(bytes) => bytes,
			SharedBytes::ByArc(ref bytes) => &**bytes,
		}
	}
}
impl<'a> From<&'a [u8]> for SharedBytes<'a> {
	fn from(bytes: &'a [u8]) -> SharedBytes<'a> {
		SharedBytes::ByRef(bytes)
	}
}
impl From<Arc<[u8]>> for SharedBytes<'static> {
	fn from(bytes: Arc<[u8]>) -> SharedBytes<'static> {
		SharedBytes::ByArc(bytes)
	}
}
impl From<Box<[u8]>> for SharedBytes<'static> {
	fn from(bytes: Box<[u8]>) -> SharedBytes<'static> {
		SharedBytes::ByArc(bytes.into())
	}
}
impl From<Vec<u8>> for SharedBytes<'static> {
	fn from(bytes: Vec<u8>) -> SharedBytes<'static> {
		SharedBytes::ByArc(bytes.into())
	}
}

/// Represents a Unicode code point.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Codepoint(pub u32);
/// Represents a glyph identifier for a particular font. This identifier will not necessarily
/// correspond to the correct glyph in a font other than the one that it was obtained from.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct GlyphId(pub u32);
/// A single glyph of a font. this is a thin wrapper referring to the font,
/// glyph id and scaling information.
#[derive(Clone)]
pub struct Glyph<'a> {
	inner: GlyphInner<'a>,
	v: Vec2
}

#[derive(Clone)]
struct GlyphInner<'a>(Font<'a>, u32);

/// The "horizontal metrics" of a glyph. This is useful for calculating the
/// horizontal offset of a glyph from the previous one in a string when laying a
/// string out horizontally.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
struct HMetrics {
	/// The horizontal offset that the origin of the next glyph should be from
	/// the origin of this glyph.
	pub advance_width: f32,
	/// The horizontal offset between the origin of this glyph and the leftmost
	/// edge/point of the glyph.
	pub left_side_bearing: f32,
}
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd)]
/// The "vertical metrics" of a font at a particular scale. This is useful for
/// calculating the amount of vertical space to give a line of text, and for
/// computing the vertical offset between successive lines.
struct VMetrics {
	/// The highest point that any glyph in the font extends to above the
	/// baseline. Typically positive.
	pub ascent: f32,
	/// The lowest point that any glyph in the font extends to below the
	/// baseline. Typically negative.
	pub descent: f32,
	/// The gap to leave between the descent of one line and the ascent of the
	/// next. This is of course only a guideline given by the font's designers.
	pub line_gap: f32,
}

impl From<tt::VMetrics> for VMetrics {
	fn from(vm: tt::VMetrics) -> Self {
		Self {
			ascent: vm.ascent as f32,
			descent: vm.descent as f32,
			line_gap: vm.line_gap as f32,
		}
	}
}
/// A trait for types that can be converted into a `GlyphId`, in the context of
/// a specific font.
///
/// Many `fonterator` functions that operate on characters accept values of any
/// type that implements `IntoGlyphId`. Such types include `char`, `Codepoint`,
/// and obviously `GlyphId` itself.
trait IntoGlyphId {
	/// Convert `self` into a `GlyphId`, consulting the index map of `font` if
	/// necessary.
	fn into_glyph_id(self, &Font) -> GlyphId;
}
impl IntoGlyphId for char {
	fn into_glyph_id(self, font: &Font) -> GlyphId {
		GlyphId(font.info.find_glyph_index(self as u32))
	}
}
impl IntoGlyphId for Codepoint {
	fn into_glyph_id(self, font: &Font) -> GlyphId {
		GlyphId(font.info.find_glyph_index(self.0))
	}
}
impl IntoGlyphId for GlyphId {
	fn into_glyph_id(self, _font: &Font) -> GlyphId {
		self
	}
}
impl<'a> FontCollection<'a> {
	/// Constructs a font collection from an array of bytes, typically loaded
	/// from a font file, which may be a single font or a TrueType Collection
	/// holding a number of fonts. This array may be owned (e.g. `Vec<u8>`), or
	/// borrowed (`&[u8]`). As long as `From<T>` is implemented for `Bytes` for
	/// some type `T`, `T` can be used as input.
	///
	/// This returns an error if `bytes` does not seem to be font data in a
	/// format we recognize.
	pub fn new<B: Into<SharedBytes<'a>>>(bytes: B) -> Result<FontCollection<'a>, Error> {
		let bytes = bytes.into();
		// We should use tt::is_collection once it lands in stb_truetype-rs:
		// https://github.com/redox-os/stb_truetype-rs/pull/15
		if !tt::is_font(&bytes) && &bytes[0..4] != b"ttcf" {
			return Err(Error::UnrecognizedFormat);
		}

		Ok(FontCollection(bytes))
	}
	/// If this `FontCollection` holds a single font, or a TrueType Collection
	/// containing only one font, return that as a `Font`. The `FontCollection`
	/// is consumed.
	///
	/// If this `FontCollection` holds multiple fonts, return a
	/// `CollectionContainsMultipleFonts` error.
	///
	/// If an error occurs, the `FontCollection` is lost, since this function
	/// takes ownership of it, and the error values don't give it back. If that
	/// is a problem, use the `font_at` or `into_fonts` methods instead, which
	/// borrow the `FontCollection` rather than taking ownership of it.
	pub fn into_font(self) -> Result<Font<'a>, Error> {
		let offset = if tt::is_font(&self.0) {
			0
		} else if tt::get_font_offset_for_index(&self.0, 1).is_some() {
			return Err(Error::CollectionContainsMultipleFonts);
		} else {
			// We now know that either a) `self.0` is a collection with only one
			// font, or b) `get_font_offset_for_index` found data it couldn't
			// recognize. Request the first font's offset, distinguishing
			// those two cases.
			match tt::get_font_offset_for_index(&self.0, 0) {
				None => return Err(Error::IllFormed),
				Some(offset) => offset,
			}
		};
		let info = tt::FontInfo::new(self.0, offset as usize).ok_or(Error::IllFormed)?;
		Ok(Font { info })
	}
	/// Gets the font at index `i` in the font collection, if it exists and is
	/// valid. The produced font borrows the font data that is either borrowed
	/// or owned by this font collection.
	pub fn font_at(&self, i: usize) -> Result<Font<'a>, Error> {
		let offset = tt::get_font_offset_for_index(&self.0, i as i32)
			.ok_or(Error::CollectionIndexOutOfBounds)?;
		let info = tt::FontInfo::new(self.0.clone(), offset as usize).ok_or(Error::IllFormed)?;
		Ok(Font { info })
	}
	/// Converts `self` into an `Iterator` yielding each `Font` that exists
	/// within the collection.
	pub fn into_fonts(self) -> Vec<Font<'a>> {
		let mut fonts = vec![];
		let mut index = 0;

		loop {
			let result = self.font_at(index);
			if let Err(Error::CollectionIndexOutOfBounds) = result {
				break
			}
			index += 1;
			fonts.push(result.unwrap());
		}

		fonts
	}
}

/// An iterator over glyphs in a string.
pub struct GlyphIterator<'a> {
	// The font
	font: &'a Font<'a>,
	// Scaling info
	api_scale: (f32, f32),
	// ...
	scale: Vec2,
	// Normalized string
	string: Vec<char>,
	// Which character in the string
	cursor: usize,
	// The previous glyph
	last: Option<Glyph<'a>>,
}

impl<'a> Iterator for GlyphIterator<'a> {
	type Item = (Glyph<'a>, f32);

	fn next(&mut self) -> Option<(Glyph<'a>, f32)> {
		let c = self.string.get(self.cursor);

		if let Some(c) = c {
			let glyph: Glyph<'a> = self.font.glyph(*c, self.scale);
			let mut advance = self.font.info
				.get_glyph_h_metrics(glyph.id().0)
				.advance_width as f32 * self.scale.0;

			if self.cursor != 0 {
				advance += self.font.kerning(self.api_scale,
					self.scale, self.last.as_ref().unwrap(),
					&glyph);
			}

			self.last = Some(glyph.clone());
			self.cursor += 1;
			Some((glyph, advance))
		} else {
			None
		}
	}
}

impl<'a> Font<'a> {
	/// Constructs a font from an array of bytes, this is a shortcut for
	/// `FontCollection::new` for collections comprised of a single font.
	pub fn new<B: Into<SharedBytes<'a>>>(bytes: B) -> Result<Font<'a>, Error> {
		FontCollection::new(bytes).and_then(|c| c.into_font())
	}

	/// The "vertical metrics" for this font at a given scale. These metrics are
	/// shared by all of the glyphs in the font. See `VMetrics` for more detail.
	fn v_metrics(&self, scale: Vec2) -> f32 {
		let vm = self.info.get_v_metrics();
		let scale = scale.1;
		(vm.ascent as f32) * scale
	}

	/// Returns the units per EM square of this font
	pub fn units_per_em(&self) -> u16 {
		self.info.units_per_em()
	}

	/// The number of glyphs present in this font. Glyph identifiers for this
	/// font will always be in the range `0..self.glyph_count()`
	pub fn glyph_count(&self) -> usize {
		self.info.get_num_glyphs() as usize
	}

	/// Returns the corresponding glyph for a Unicode code point or a glyph id
	/// for this font.
	///
	/// If `id` is a `GlyphId`, it must be valid for this font; otherwise, this
	/// function panics. `GlyphId`s should always be produced by looking up some
	/// other sort of designator (like a Unicode code point) in a font, and
	/// should only be used to index the font they were produced for.
	///
	/// Note that code points without corresponding glyphs in this font map to
	/// the ".notdef" glyph, glyph 0.
	fn glyph<C: IntoGlyphId>(&self, id: C, v: Vec2) -> Glyph<'a> {
		let gid = id.into_glyph_id(self);
		assert!((gid.0 as usize) < self.glyph_count());
		// font clone either a reference clone, or arc clone
		Glyph::new(GlyphInner(self.clone(), gid.0), v)
	}
	/// Returns an iterator over the names for this font.
	pub fn font_name_strings(&self) -> tt::FontNameIter<SharedBytes<'a>> {
		self.info.get_font_name_strings()
	}
	/// Returns additional kerning to apply as well as that given by HMetrics
	/// for a particular pair of glyphs.
	fn pair_kerning<A, B>(&self, scale: (f32, f32), v: Vec2, first: A, second: B) -> f32
	where
		A: IntoGlyphId,
		B: IntoGlyphId,
	{
		let (first, second) = (self.glyph(first, v), self.glyph(second, v));
		let factor = self.info.scale_for_pixel_height(scale.1) * (scale.0 / scale.1);
		let kern = self.info
			.get_glyph_kern_advance(first.id().0, second.id().0);
		factor * kern as f32
	}
	/// Get an iterator over the glyphs in a string.
	pub fn glyphs<T: ToString>(&'a self, text: T, scale: (f32, f32))
		-> GlyphIterator<'a>
	{
		let (scale_x, scale_y) = {
			let scale_y = self.info.scale_for_pixel_height(scale.1);
			let scale_x = scale_y * scale.0 / scale.1;
			(scale_x, scale_y)
		};

		GlyphIterator {
			font: &self,
			api_scale: scale,
			scale: Vec2(scale_x, scale_y),
			string: text.to_string().nfc().collect::<Vec<char>>(),
			cursor: 0,
			last: None,
		}
	}
	/// Get the proper spacing from the start of one character to the next.
	fn kerning(&self, scale: (f32, f32), v: Vec2, first: &Glyph<'a>,
		second: &Glyph<'a>) -> f32
	{
		self.pair_kerning(scale, v, first.id(), second.id())
	}
}
impl<'a> Glyph<'a> {
	fn new(inner: GlyphInner<'a>, v: Vec2) -> Glyph<'a> {
		Glyph { inner, v }
	}
	/// The font to which this glyph belongs.
	fn font(&self) -> &Font<'a> {
		&self.inner.0
	}
	/// The glyph identifier for this glyph.
	fn id(&self) -> GlyphId {
		GlyphId(self.inner.1)
	}
	/// Convert the glyph to an iterator over PathOps
	pub fn draw(&self, point_x: f32, mut point_y: f32) -> Path {
		use stb_truetype::VertexType;
		point_y += self.font().v_metrics(self.v);
		let shape = {
			let (font, id) = (self.font(), self.id());

			font.info.get_glyph_shape(id.0).unwrap_or_else(Vec::new)
		};
		let mut path = Vec::new();
		let mut origin = (0.0, 0.0);
		for v in shape {
			let x = v.x as f32 * self.v.0 + point_x;
			let y = -v.y as f32 * self.v.1 + point_y;

			match v.vertex_type() {
				VertexType::LineTo => {
					if x == origin.0 && y == origin.1 {
						path.push(PathOp::LineClose);
					} else {
						path.push(PathOp::LineTo(x, y));
					}
				}
				VertexType::CurveTo => {
					let cx = v.cx as f32 * self.v.0
						+ point_x;
					let cy = -v.cy as f32 * self.v.1
						+ point_y;

					if x == origin.0 && y == origin.1 {
						path.push(PathOp::QuadClose(
							cx, cy));
					} else {
						path.push(PathOp::QuadTo(
							x, y, cx, cy));
					}
				}
				VertexType::MoveTo => {
					path.push(PathOp::MoveTo(x, y));
					origin = (x, y);
				}
			}
		}

		Path(path)
	}
}

/// The type for errors returned by Fonterator.
#[derive(Debug)]
pub enum Error {
	/// Font data presented to Fonterator is not in a format that the
	/// library recognizes.
	UnrecognizedFormat,

	/// Font data presented to Fonterator was ill-formed (lacking necessary
	/// tables, for example).
	IllFormed,

	/// The caller tried to access the `i`'th font from a `FontCollection`,
	/// but the collection doesn't contain that many fonts.
	CollectionIndexOutOfBounds,

	/// The caller tried to convert a `FontCollection` into a font via
	/// `into_font`, but the `FontCollection` contains more than one font.
	CollectionContainsMultipleFonts,
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> std::result::Result<(), fmt::Error> {
		f.write_str(std::error::Error::description(self))
	}
}

impl std::error::Error for Error {
	fn description(&self) -> &str {
		use self::Error::*;
		match *self {
			UnrecognizedFormat => "Font data in unrecognized format",
			IllFormed => "Font data is ill-formed",
			CollectionIndexOutOfBounds => "Font collection has no font at the given index",
			CollectionContainsMultipleFonts => {
				"Attempted to convert collection into a font, \
				 but collection contais more than one font"
			}
		}
	}
}

impl std::convert::From<Error> for std::io::Error {
	fn from(error: Error) -> Self {
		std::io::Error::new(std::io::ErrorKind::Other, error)
	}
}
