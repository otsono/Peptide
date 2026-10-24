// Flash's other classes the source's code uses.

// a map keyed by any value: a string or number by its text, an object by itself
// class Dictionary
var Dictionary = { name: "Dictionary" };
Dictionary.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("Dictionary");
	me.__dict = true;
	me.__s = new haxe.ds.StringMap();
	me.__o = new haxe.ds.ObjectMap();
};
Dictionary.init = function(me, ?weak) {};
Dictionary.create = function(?weak) { var me = {}; Dictionary.build(me); return me; };
Dictionary.plain = function(k) { return k == null || Std.isOfType(k, String) || Std.isOfType(k, Float) || k == true || k == false; };
Dictionary.get = function(d, k) { return Dictionary.plain(k) ? d.__s.get(AS3.str(k)) : d.__o.get(k); };
Dictionary.set = function(d, k, v) { if (Dictionary.plain(k)) d.__s.set(AS3.str(k), v); else d.__o.set(k, v); return v; };
Dictionary.has = function(d, k) { return Dictionary.plain(k) ? d.__s.exists(AS3.str(k)) : d.__o.exists(k); };
Dictionary.remove = function(d, k) { return Dictionary.plain(k) ? d.__s.remove(AS3.str(k)) : d.__o.remove(k); };
Dictionary.keys = function(d) {
	var out = [];
	for (k in d.__s.keys()) out.push(k);
	for (k in d.__o.keys()) out.push(k);
	return out;
};

var Event = { ENTER_FRAME: "enterFrame", ADDED: "added", ADDED_TO_STAGE: "addedToStage", REMOVED: "removed",
	REMOVED_FROM_STAGE: "removedFromStage", COMPLETE: "complete" };

// class Error
var Error = { name: "Error" };
Error.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("Error");
	me.toString = function() { return "Error: " + AS3.str(me.message); };
};
Error.init = function(me, ?msg, ?id) { me.message = msg == null ? "" : msg; me.errorID = id == null ? 0 : id; me.name = "Error"; };
Error.create = function(?msg, ?id) { var me = {}; Error.build(me); Error.init(me, msg, id); return me; };

// a picture drawn into by code: this engine draws no pixels, so it keeps its size and says a
// drawing call is a gap
// class BitmapData
var BitmapData = { name: "BitmapData" };
BitmapData.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("BitmapData");
	me.fillRect = function(r, c) { PeptideRuntime.gap("BitmapData.fillRect: no pixel drawing here"); };
	me.setPixel = function(x, y, c) { PeptideRuntime.gap("BitmapData.setPixel: no pixel drawing here"); };
	me.setPixel32 = me.setPixel;
	me.getPixel = function(x, y) { return me.fill; };
	me.getPixel32 = me.getPixel;
	me.copyPixels = function(src, r, p, ?a, ?ap, ?merge) { PeptideRuntime.gap("BitmapData.copyPixels: no pixel drawing here"); };
	me.draw = function(src, ?m, ?ct, ?blend, ?clip, ?smooth) { PeptideRuntime.gap("BitmapData.draw: no pixel drawing here"); };
	me.colorTransform = function(r, ct) { PeptideRuntime.gap("BitmapData.colorTransform: no pixel drawing here"); };
	me.lock = function() {};
	me.unlock = function(?r) {};
	me.dispose = function() {};
	me.clone = function() { return BitmapData.create(me.width, me.height, me.transparent, me.fill); };
};
BitmapData.init = function(me, w, h, ?transparent, ?fill) {
	me.width = w; me.height = h;
	me.transparent = transparent == null ? true : transparent;
	me.fill = fill == null ? 0xFFFFFFFF : fill;
	me.rect = Rectangle.create(0, 0, w, h);
};
BitmapData.create = function(w, h, ?transparent, ?fill) { var me = {}; BitmapData.build(me); BitmapData.init(me, w, h, transparent, fill); return me; };

// class Bitmap
var Bitmap = { name: "Bitmap" };
Bitmap.build = function(me) {
	DisplayObject.build(me);
	me.__is.push("Bitmap");
};
Bitmap.init = function(me, ?data, ?snap, ?smooth) {
	me.bitmapData = data;
	me.x = 0; me.y = 0; me.scaleX = 1; me.scaleY = 1; me.rotation = 0; me.alpha = 1; me.visible = true;
	me.width = data == null ? 0 : data.width;
	me.height = data == null ? 0 : data.height;
};
Bitmap.create = function(?data, ?snap, ?smooth) { var me = {}; Bitmap.build(me); Bitmap.init(me, data, snap, smooth); return me; };

// a date: this engine has no clock (a match runs the same on every machine), so "now" is one fixed
// moment, noon on the first of the year, said once as a gap. a date made from numbers is exact
// class Date
var Date = { name: "Date" };
Date.now = 1767268800000.0;
Date.days = function(y, m, d) {
	// days since 1970-01-01 of a calendar day (m from 0; a month past 11 rolls into the next year)
	y = y + Math.floor(m / 12);
	m = m - 12 * Math.floor(m / 12);
	var yy = m < 2 ? y - 1 : y;
	var era = Math.floor(yy / 400);
	var yoe = yy - era * 400;
	var mp = (m + 10) % 12;
	var doy = Math.floor((153 * mp + 2) / 5) + d - 1;
	var doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
	return era * 146097 + doe - 719468;
};
Date.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("Date");
	me.parts = function() {
		var z = Math.floor(me.__t / 86400000) + 719468;
		var era = Math.floor(z / 146097);
		var doe = z - era * 146097;
		var yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
		var doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
		var mp = Math.floor((5 * doy + 2) / 153);
		var d = doy - Math.floor((153 * mp + 2) / 5) + 1;
		var m = mp < 10 ? mp + 2 : mp - 10;
		var y = yoe + era * 400 + (m < 2 ? 1 : 0);
		var ms = me.__t - Math.floor(me.__t / 86400000) * 86400000.0;
		return [y, m, d, Math.floor(ms / 3600000), Math.floor(ms / 60000) % 60, Math.floor(ms / 1000) % 60, ms % 1000];
	};
	me.set = function(i, v) {
		var p = me.parts();
		p[i] = AS3.num(v);
		me.__t = Date.days(p[0], p[1], p[2]) * 86400000.0 + p[3] * 3600000 + p[4] * 60000 + p[5] * 1000 + p[6];
		return me.__t;
	};
	me.getTime = function() { return me.__t; };
	me.setTime = function(t) { me.__t = AS3.num(t); return me.__t; };
	me.getFullYear = function() { return me.parts()[0]; };
	me.getMonth = function() { return me.parts()[1]; };
	me.getDate = function() { return me.parts()[2]; };
	me.getHours = function() { return me.parts()[3]; };
	me.getMinutes = function() { return me.parts()[4]; };
	me.getSeconds = function() { return me.parts()[5]; };
	me.getMilliseconds = function() { return me.parts()[6]; };
	me.getDay = function() { var d = Math.floor(me.__t / 86400000); return ((d + 4) % 7 + 7) % 7; };
	me.setFullYear = function(v) { return me.set(0, v); };
	me.setMonth = function(v) { return me.set(1, v); };
	me.setDate = function(v) { return me.set(2, v); };
	me.setHours = function(v) { return me.set(3, v); };
	me.setMinutes = function(v) { return me.set(4, v); };
	me.setSeconds = function(v) { return me.set(5, v); };
	me.setMilliseconds = function(v) { return me.set(6, v); };
	me.getUTCHours = me.getHours; me.getUTCMinutes = me.getMinutes; me.getUTCFullYear = me.getFullYear;
	me.getUTCMonth = me.getMonth; me.getUTCDate = me.getDate; me.getUTCDay = me.getDay;
	me.getTimezoneOffset = function() { return 0; };
	me.valueOf = me.getTime;
};
Date.init = function(me, ?a, ?b, ?c, ?d, ?e, ?f, ?g) {
	if (a == null) { PeptideRuntime.gap("Date: the machine's clock (now is a fixed moment here)"); me.__t = Date.now; return; }
	// another date: the same moment
	if (b == null) { me.__t = (!Std.isOfType(a, Float) && !Std.isOfType(a, String) && a.__t != null) ? a.__t : AS3.num(a); return; }
	me.__t = Date.days(AS3.num(a), AS3.num(b), c == null ? 1 : AS3.num(c)) * 86400000.0
		+ (d == null ? 0 : AS3.num(d)) * 3600000 + (e == null ? 0 : AS3.num(e)) * 60000 + (f == null ? 0 : AS3.num(f)) * 1000 + (g == null ? 0 : AS3.num(g));
};
Date.create = function(?a, ?b, ?c, ?d, ?e, ?f, ?g) { var me = {}; Date.build(me); Date.init(me, a, b, c, d, e, f, g); return me; };

// a pattern: this engine's scripts have no regular expressions, so one is matched as the text it
// is; a pattern that uses the syntax is a gap
// class RegExp
var RegExp = { name: "RegExp" };
RegExp.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("RegExp");
	me.__regexp = true;
	me.test = function(s) { return AS3.str(s).indexOf(me.source) >= 0; };
};
RegExp.init = function(me, ?src, ?flags) {
	me.source = src == null ? "" : AS3.str(src);
	var f = flags == null ? "" : AS3.str(flags);
	me.global = f.indexOf("g") >= 0;
	me.ignoreCase = f.indexOf("i") >= 0;
	var special = "\\^$.|?*+()[]{}";
	for (i in 0...me.source.length) if (special.indexOf(me.source.charAt(i)) >= 0) { PeptideRuntime.gap("RegExp: a pattern beyond plain text (" + me.source + ")"); break; }
};
RegExp.create = function(?src, ?flags) { var me = {}; RegExp.build(me); RegExp.init(me, src, flags); return me; };
