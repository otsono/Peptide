// Flash's geometry classes, as the source's code uses them: plain fields, so a read of `.x` costs
// nothing more than a field (no accessor is defined for them on purpose).

// class Point
var Point = { name: "Point" };
Point.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("Point");
	me.x = 0;
	me.y = 0;
	me.clone = function() { return Point.create(me.x, me.y); };
	me.__copy = me.clone;
	me.add = function(p) { return Point.create(me.x + p.x, me.y + p.y); };
	me.subtract = function(p) { return Point.create(me.x - p.x, me.y - p.y); };
	me.offset = function(dx, dy) { me.x += dx; me.y += dy; };
	me.equals = function(p) { return p != null && p.x == me.x && p.y == me.y; };
	me.normalize = function(n) { var l = Math.sqrt(me.x * me.x + me.y * me.y); if (l > 0) { me.x = me.x * n / l; me.y = me.y * n / l; } };
};
Point.init = function(me, ?x, ?y) { me.x = x == null ? 0 : x; me.y = y == null ? 0 : y; };
Point.create = function(?x, ?y) { var me = {}; Point.build(me); Point.init(me, x, y); return me; };
Point.distance = function(a, b) { var dx = a.x - b.x; var dy = a.y - b.y; return Math.sqrt(dx * dx + dy * dy); };
Point.polar = function(len, ang) { return Point.create(len * Math.cos(ang), len * Math.sin(ang)); };
Point.interpolate = function(a, b, f) { return Point.create(b.x + (a.x - b.x) * f, b.y + (a.y - b.y) * f); };

// class Rectangle
var Rectangle = { name: "Rectangle" };
Rectangle.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("Rectangle");
	me.clone = function() { return Rectangle.create(me.x, me.y, me.width, me.height); };
	me.__copy = me.clone;
	me.contains = function(px, py) { return px >= me.x && py >= me.y && px < me.x + me.width && py < me.y + me.height; };
	me.containsPoint = function(p) { return me.contains(p.x, p.y); };
	me.intersects = function(r) { return r.x < me.x + me.width && me.x < r.x + r.width && r.y < me.y + me.height && me.y < r.y + r.height; };
	me.offset = function(dx, dy) { me.x += dx; me.y += dy; };
	me.isEmpty = function() { return me.width <= 0 || me.height <= 0; };
	// the area both cover, empty (all zero) when they do not meet
	me.intersection = function(r) {
		var x0 = Math.max(me.x, r.x);
		var y0 = Math.max(me.y, r.y);
		var x1 = Math.min(me.x + me.width, r.x + r.width);
		var y1 = Math.min(me.y + me.height, r.y + r.height);
		return (x1 <= x0 || y1 <= y0) ? Rectangle.create() : Rectangle.create(x0, y0, x1 - x0, y1 - y0);
	};
};
Rectangle.init = function(me, ?x, ?y, ?w, ?h) {
	me.x = x == null ? 0 : x;
	me.y = y == null ? 0 : y;
	me.width = w == null ? 0 : w;
	me.height = h == null ? 0 : h;
};
Rectangle.create = function(?x, ?y, ?w, ?h) { var me = {}; Rectangle.build(me); Rectangle.init(me, x, y, w, h); return me; };

// class ColorTransform
var ColorTransform = { name: "ColorTransform" };
ColorTransform.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("ColorTransform");
	me.__copy = function() { return ColorTransform.create(me.redMultiplier, me.greenMultiplier, me.blueMultiplier, me.alphaMultiplier, me.redOffset, me.greenOffset, me.blueOffset, me.alphaOffset); };
};
ColorTransform.init = function(me, ?rm, ?gm, ?bm, ?am, ?ro, ?go, ?bo, ?ao) {
	me.redMultiplier = rm == null ? 1 : rm;
	me.greenMultiplier = gm == null ? 1 : gm;
	me.blueMultiplier = bm == null ? 1 : bm;
	me.alphaMultiplier = am == null ? 1 : am;
	me.redOffset = ro == null ? 0 : ro;
	me.greenOffset = go == null ? 0 : go;
	me.blueOffset = bo == null ? 0 : bo;
	me.alphaOffset = ao == null ? 0 : ao;
};
ColorTransform.create = function(?rm, ?gm, ?bm, ?am, ?ro, ?go, ?bo, ?ao) {
	var me = {};
	ColorTransform.build(me);
	ColorTransform.init(me, rm, gm, bm, am, ro, go, bo, ao);
	return me;
};

// a display object's transform: its colour transform tints the engine's object it stands for
// class Transform
var Transform = { name: "Transform" };
Transform.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("Transform");
	me.__ct = null;
	me.get_colorTransform = function() { return me.__ct == null ? ColorTransform.create() : me.__ct; };
	me.set_colorTransform = function(ct) { me.__ct = ct; if (ct != null) Visuals.tint(me.__owner, ct); return ct; };
};
Transform.init = function(me, ?owner) { me.__owner = owner; };
Transform.create = function(?owner) { var me = {}; Transform.build(me); Transform.init(me, owner); return me; };

// filters are kept as the data the source gives them; what the engine can draw of them is
// applied where a clip is the engine's object (see Visuals)
// class ColorMatrixFilter
var ColorMatrixFilter = { name: "ColorMatrixFilter" };
ColorMatrixFilter.build = function(me) { if (me.__is == null) me.__is = []; me.__is.push("ColorMatrixFilter"); };
ColorMatrixFilter.init = function(me, ?m) { me.matrix = m; };
ColorMatrixFilter.create = function(?m) { var me = {}; ColorMatrixFilter.build(me); ColorMatrixFilter.init(me, m); return me; };
// class BlurFilter
var BlurFilter = { name: "BlurFilter" };
BlurFilter.build = function(me) { if (me.__is == null) me.__is = []; me.__is.push("BlurFilter"); };
BlurFilter.init = function(me, ?bx, ?by, ?quality) { me.blurX = bx == null ? 4 : bx; me.blurY = by == null ? 4 : by; me.quality = quality == null ? 1 : quality; };
BlurFilter.create = function(?bx, ?by, ?quality) { var me = {}; BlurFilter.build(me); BlurFilter.init(me, bx, by, quality); return me; };
// class GlowFilter
var GlowFilter = { name: "GlowFilter" };
GlowFilter.build = function(me) { if (me.__is == null) me.__is = []; me.__is.push("GlowFilter"); };
GlowFilter.init = function(me, ?color, ?alpha, ?bx, ?by, ?strength, ?quality, ?inner, ?knockout) {
	me.color = color; me.alpha = alpha; me.blurX = bx; me.blurY = by; me.strength = strength;
	me.quality = quality; me.inner = inner; me.knockout = knockout;
};
GlowFilter.create = function(?color, ?alpha, ?bx, ?by, ?strength, ?quality, ?inner, ?knockout) {
	var me = {};
	GlowFilter.build(me);
	GlowFilter.init(me, color, alpha, bx, by, strength, quality, inner, knockout);
	return me;
};

// this engine has no blend modes: the names only
var BlendMode = { NORMAL: "normal", ADD: "add", SCREEN: "screen", MULTIPLY: "multiply", OVERLAY: "overlay",
	HARDLIGHT: "hardlight", LIGHTEN: "lighten", DARKEN: "darken", DIFFERENCE: "difference", SUBTRACT: "subtract",
	ALPHA: "alpha", ERASE: "erase", INVERT: "invert", LAYER: "layer" };
