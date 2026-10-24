// AS3's own rules, on Fraymakers' script interpreter.
//
// The compiler writes plain Haxe wherever the two languages agree and calls one of these where
// they do not. Each one is the rule AS3 states, measured against this interpreter:
//
//   a test          only a literal true passes here; AS3 tests truthiness (0, "", null and NaN
//                   are false, anything else true)
//   <, <=, >, >=    here a null or a NaN compares like a number would not in AS3
//   >>>             here it is signed
//   o[key]          here a name worked out at run time reads nothing on an object; AS3 reads the
//                   property. keys like that live in the object's own key map (`__keys`), which
//                   is a map made with `new`: this interpreter finds a class by its path only there
//   a[-1]           here it stops the script; in AS3 it is undefined
//   is, as          types are named, since most of AS3's type names are no value here
//
// Everything is on one object, so a converted script reads `AS3.truthy(x)` and a name can never
// collide with the content's own.

var AS3 = {};

// ---- tests ----------------------------------------------------------------------------------------

AS3.truthy = function(v) {
	if (v == null) return false;
	if (v == true) return true;
	if (v == false) return false;
	if (Std.isOfType(v, String)) return v != "";
	if (Std.isOfType(v, Float)) return v != 0 && !Math.isNaN(v);
	return true;
};

// ---- numbers ---------------------------------------------------------------------------------------

AS3.num = function(v) {
	if (v == null) return 0;
	if (v == true) return 1;
	if (v == false) return 0;
	if (Std.isOfType(v, Float)) return v;
	if (Std.isOfType(v, String)) return v == "" ? 0 : Std.parseFloat(v);
	return Math.NaN;
};

AS3.int = function(v) {
	var n = AS3.num(v);
	if (Math.isNaN(n) || n == Math.POSITIVE_INFINITY || n == Math.NEGATIVE_INFINITY) return 0;
	// truncate toward zero, then wrap to 32 bits as AS3 does (by arithmetic: this interpreter's
	// floor and ceil give a 32-bit int, which overflows before the wrap)
	var t = n - (n % 1);
	var m = t % 4294967296.0;
	if (m >= 2147483648.0) m -= 4294967296.0;
	if (m < -2147483648.0) m += 4294967296.0;
	// a large one stays a whole Float: this interpreter compares two ints by their difference,
	// which overflows past 2^31 (0 > int.MIN_VALUE answers false), and a Float compares right.
	// world tournament's leader board starts its best score at int.MIN_VALUE and found no leader
	if (m >= 1073741824.0 || m < -1073741824.0) return m;
	return Std.int(m);
};

AS3.uint = function(v) {
	var i = AS3.int(v);
	return i < 0 ? i + 4294967296.0 : i;
};

AS3.ushr = function(a, b) {
	var s = AS3.int(b) & 31;
	var q = AS3.uint(a) / Math.pow(2, s);
	return q - (q % 1);
};

// AS3's comparison: two strings compare as strings, anything else as numbers, and a NaN makes it
// false whichever way it is asked
AS3.compare = function(a, b) {
	if (Std.isOfType(a, String) && Std.isOfType(b, String)) return a < b ? -1 : (a > b ? 1 : 0);
	var x = AS3.num(a);
	var y = AS3.num(b);
	if (Math.isNaN(x) || Math.isNaN(y)) return null;
	return x < y ? -1 : (x > y ? 1 : 0);
};
// two numbers, the common case (a loop's test), go straight to the comparison: what `compare`
// would answer for them, without its conversions (krazoa palace runs one over a picture a frame)
AS3.lt = function(a, b) { if (Std.isOfType(a, Float) && Std.isOfType(b, Float)) return !Math.isNaN(a) && !Math.isNaN(b) && a < b; var c = AS3.compare(a, b); return c != null && c < 0; };
AS3.le = function(a, b) { if (Std.isOfType(a, Float) && Std.isOfType(b, Float)) return !Math.isNaN(a) && !Math.isNaN(b) && a <= b; var c = AS3.compare(a, b); return c != null && c <= 0; };
AS3.gt = function(a, b) { if (Std.isOfType(a, Float) && Std.isOfType(b, Float)) return !Math.isNaN(a) && !Math.isNaN(b) && a > b; var c = AS3.compare(a, b); return c != null && c > 0; };
AS3.ge = function(a, b) { if (Std.isOfType(a, Float) && Std.isOfType(b, Float)) return !Math.isNaN(a) && !Math.isNaN(b) && a >= b; var c = AS3.compare(a, b); return c != null && c >= 0; };

AS3.random = function() { return Random.getFloat(0, 1); };

// ---- strings --------------------------------------------------------------------------------------

// a value as AS3 writes it: an array its elements joined by commas, an object its own
// `toString` or `[object Name]`. an object is never walked (the engine's own conversion follows
// every field, and a loop through a clip's parent and children overflows it)
AS3.str = function(v) {
	if (v == null) return "null";
	if (Std.isOfType(v, String) || Std.isOfType(v, Float) || Std.isOfType(v, Bool)) return Std.string(v);
	if (v.__is == null && v.__type == null) {
		if (AS3.isArray(v)) { var out = []; for (x in v) out.push(x == null ? "" : AS3.str(x)); return out.join(","); }
		return "[object Object]";
	}
	if (v.toString != null) return AS3.str(v.toString());
	var t = v.__type != null ? AS3.str(v.__type) : AS3.str(v.__is[v.__is.length - 1]);
	var dot = t.lastIndexOf(".");
	return "[object " + (dot >= 0 ? t.substr(dot + 1) : t) + "]";
};

// the name of a value's class (`getQualifiedClassName`): a library symbol's own linkage name
AS3.className = function(v) {
	if (v == null) return "null";
	if (Std.isOfType(v, String)) return "String";
	if (Std.isOfType(v, Bool)) return "Boolean";
	if (Std.isOfType(v, Float)) return v == Std.int(v) ? "int" : "Number";
	if (v.__linkage != null) return v.__linkage;
	if (v.__is != null && v.__is.length > 0) return v.__is[v.__is.length - 1];
	return AS3.isArray(v) ? "Array" : "Object";
};

// a library class asked for by its name (`getDefinitionByName`): made as the library makes it, a
// bitmap's data for a bitmap, else its clip
AS3.definition = function(n) {
	var name = AS3.str(n);
	var bmps = PeptideRuntime.data.bitmaps;
	if (bmps != null) for (b in bmps) if (b.name == name) {
		return { name: name, create: function(?w, ?h) { var d = BitmapData.create(b.w, b.h, true, 0); d.__linkage = name; return d; } };
	}
	return { name: name, create: function() { return PeptideAPI.clipByLinkage(name); } };
};

// trace: its values joined by spaces, to the report
AS3.trace = function(args) {
	var out = [];
	for (a in args) out.push(AS3.str(a));
	PeptideRuntime.say("trace: " + out.join(" "));
};

// ---- properties by a name worked out at run time -------------------------------------------------

AS3.at = function(o, k) {
	if (o == null) return null;
	// an array by a number, the common case, before the lookups a keyed object needs
	if (o.push != null && Std.isOfType(k, Float)) return (k >= 0 && k < o.length) ? o[Std.int(k)] : null;
	if (o.__dict == true) return Dictionary.get(o, k);
	if (o.__shape != null) return o.__shape.get(o, AS3.str(k));
	if (Std.isOfType(k, Float)) {
		if (Std.isOfType(o, String)) return (k >= 0 && k < o.length) ? o.charAt(k) : null;
		if (o.push != null) return (k >= 0 && k < o.length) ? o[Std.int(k)] : null;
	}
	// an object that knows its own keys (the data a conversion writes) answers for itself
	if (o.__get != null) return o.__get(AS3.str(k));
	var keys = o.__keys;
	var v = keys == null ? null : keys.get(AS3.str(k));
	// an engine object asked for an API call by a name built as the code runs: the call, as the
	// conversion writes it, made on this object
	if (v == null && o.__fm != null && PeptideRuntime.data != null && PeptideRuntime.data.api != null) {
		var f = PeptideRuntime.data.api.get(AS3.str(k));
		if (f != null) return function(?a0, ?a1, ?a2, ?a3) { return f(o, a0, a1, a2, a3); };
	}
	// a clip's named children are its properties: `clip["region" + i]`
	if (v == null && o.__kids != null) v = o.getChildByName(AS3.str(k));
	return v;
};

AS3.put = function(o, k, v) {
	if (o.__dict == true) return Dictionary.set(o, k, v);
	if (o.__shape != null) { o.__shape.set(o, AS3.str(k), v); return v; }
	if (Std.isOfType(k, Float) && o.push != null) { o[Std.int(k)] = v; return v; }
	if (o.__set != null) { o.__set(AS3.str(k), v); return v; }
	return AS3.putKey(o, k, v);
};
AS3.putKey = function(o, k, v) {
	if (Std.isOfType(k, Float) && o.push != null) { o[Std.int(k)] = v; return v; }
	if (o.__keys == null) o.__keys = new haxe.ds.StringMap();
	o.__keys.set(AS3.str(k), v);
	return v;
};

AS3.has = function(o, k) {
	if (o == null) return false;
	if (o.__dict == true) return Dictionary.has(o, k);
	if (o.__shape != null) return o.__shape.names.indexOf(AS3.str(k)) >= 0 || (o.__keys != null && o.__keys.exists(AS3.str(k)));
	if (Std.isOfType(k, Float) && o.push != null) return k >= 0 && k < o.length;
	if (o.__get != null) return o.__get(AS3.str(k)) != null;
	return o.__keys != null && o.__keys.exists(AS3.str(k));
};

AS3.remove = function(o, k) {
	if (o == null) return false;
	if (o.__dict == true) return Dictionary.remove(o, k);
	if (Std.isOfType(k, Float) && o.push != null) { if (k >= 0 && k < o.length) o[Std.int(k)] = null; return true; }
	return o.__keys != null && o.__keys.remove(AS3.str(k));
};

// an object literal whose keys are worked out at run time: key, value, key, value, ...
AS3.object = function(pairs) {
	var o = {};
	var i = 0;
	while (i < pairs.length) { AS3.put(o, pairs[i], pairs[i + 1]); i += 2; }
	return o;
};

// `new Array(n)` makes n empty places; `new Array(a, b)` holds a and b
AS3.array = function(args) {
	if (args.length == 1 && Std.isOfType(args[0], Float)) {
		var a = [];
		var n = Std.int(args[0]);
		var i = 0;
		while (i < n) { a.push(null); i++; }
		return a;
	}
	return args;
};

// ---- enumeration (for in / for each) -------------------------------------------------------------

// the keys enumeration visits, taken once when it starts: an array's indexes, a map's keys, the
// keys an object was given at run time
AS3.keysOf = function(o) {
	var out = [];
	if (o == null) return out;
	if (o.push != null) { var i = 0; while (i < o.length) { out.push(i); i++; } return out; }
	if (o.__dict == true) return Dictionary.keys(o);
	if (o.__shape != null) for (k in o.__shape.names) out.push(k);
	if (o.__names != null) for (k in o.__names) out.push(k);
	if (o.__keys != null) for (k in o.__keys.keys()) out.push(k);
	return out;
};

// the step after `index` (0 when there is none): enumeration counts from 1. an array is walked by
// its indexes; any other object by the keys it had when the walk began, kept on the object
AS3.isArray = function(o) { return !Std.isOfType(o, String) && o.push != null; };
// an array as it is now, to change apart from the one a container holds
AS3.copyOf = function(o) {
	if (o == null) return null;
	if (o.__dict == true) {
		var d = Dictionary.create();
		for (k in o.__s.keys()) d.__s.set(k, o.__s.get(k));
		for (k in o.__o.keys()) d.__o.set(k, o.__o.get(k));
		return d;
	}
	if (Std.isOfType(o, String) || Std.isOfType(o, Float) || o == true || o == false) return o;
	if (o.__copy != null) return o.__copy();
	return AS3.isArray(o) ? o.copy() : o;
};
AS3.nextIndex = function(o, index) {
	if (o == null) return 0;
	if (AS3.isArray(o)) return index < o.length ? index + 1 : 0;
	if (index == 0) o.__enum = AS3.keysOf(o);
	return index < o.__enum.length ? index + 1 : 0;
};
AS3.nextName = function(o, index) { return AS3.isArray(o) ? index - 1 : o.__enum[index - 1]; };
AS3.nextValue = function(o, index) { return AS3.isArray(o) ? o[index - 1] : AS3.at(o, o.__enum[index - 1]); };

// ---- the language's own methods on its values, where they differ here ----------------------------

AS3.isNum = function(v) { return Std.isOfType(v, Float); };
AS3.isStr = function(v) { return Std.isOfType(v, String); };

// a number in a base (2 to 36), as AS3 writes it
AS3.radix = function(n, base) {
	var b = AS3.int(base);
	if (b == 10 || b < 2 || b > 36) return AS3.str(n);
	var v = AS3.int(n);
	if (v == 0) return "0";
	var neg = v < 0;
	var u = neg ? -v : v;
	var digits = "0123456789abcdefghijklmnopqrstuvwxyz";
	var out = "";
	while (u > 0) { var d = u % b; out = digits.charAt(d) + out; u = Std.int((u - d) / b); }
	return neg ? "-" + out : out;
};
AS3.toString = function(o, args) {
	if (o == null) return "null";
	if (AS3.isNum(o)) return args.length > 0 && args[0] != null ? AS3.radix(o, args[0]) : AS3.str(o);
	if (AS3.isStr(o)) return o;
	if (o == true || o == false) return AS3.str(o);
	return AS3.str(o);
};
AS3.toFixed = function(n, ?digits) {
	var d = digits == null ? 0 : AS3.int(digits);
	var p = Math.pow(10, d);
	var r = Math.round(AS3.num(n) * p) / p;
	var s = AS3.str(r);
	if (d <= 0) return s;
	var dot = s.indexOf(".");
	if (dot < 0) { s = s + "."; dot = s.length - 1; }
	while (s.length - dot - 1 < d) s = s + "0";
	return s;
};
// a string's or an array's part (negative ends count from the end)
AS3.slice = function(o, args) {
	if (!AS3.isStr(o) && !AS3.isArray(o) && o.slice != null) return o.slice(args[0], args[1]);
	var len = o.length;
	var a = args.length > 0 && args[0] != null ? AS3.int(args[0]) : 0;
	var b = args.length > 1 && args[1] != null ? AS3.int(args[1]) : len;
	if (a < 0) a = a + len; if (a < 0) a = 0; if (a > len) a = len;
	if (b < 0) b = b + len; if (b < 0) b = 0; if (b > len) b = len;
	if (AS3.isStr(o)) return b > a ? o.substring(a, b) : "";
	return b > a ? o.slice(a, b) : [];
};
AS3.concat = function(o, args) {
	if (AS3.isStr(o)) { var s = o; for (a in args) s = s + AS3.str(a); return s; }
	if (!AS3.isArray(o) && o.concat != null) return o.concat(args[0]);
	var out = o.copy();
	for (a in args) { if (AS3.isArray(a)) for (x in a) out.push(x); else out.push(a); }
	return out;
};
// a string's first match of a string replaced (AS3's replace with a string pattern)
AS3.replace = function(o, pat, rep) {
	if (!AS3.isStr(o)) return o.replace(pat, rep);
	// a pattern is matched as the text it is (RegExp keeps only a literal one), every match when
	// it is global
	var re = pat != null && pat.__regexp == true;
	var p = re ? pat.source : AS3.str(pat);
	var r = AS3.str(rep);
	if (re && pat.global) return p.length == 0 ? o : o.split(p).join(r);
	var i = o.indexOf(p);
	return i < 0 ? o : o.substr(0, i) + r + o.substr(i + p.length);
};
// reversed in place, and handed back (this engine's reverse gives back nothing)
AS3.reverse = function(o) {
	if (!AS3.isArray(o) && o.reverse != null) return o.reverse();
	o.reverse();
	return o;
};
AS3.forEach = function(a, fn) { var i = 0; while (i < a.length) { fn(a[i], i, a); i++; } };
AS3.some = function(a, fn) { var i = 0; while (i < a.length) { if (AS3.truthy(fn(a[i], i, a))) return true; i++; } return false; };
AS3.every = function(a, fn) { var i = 0; while (i < a.length) { if (!AS3.truthy(fn(a[i], i, a))) return false; i++; } return true; };
// AS3's sort: by a compare function, or by its options (1 case-insensitive, 2 descending,
// 16 numeric), strings by default
AS3.sort = function(a, args) {
	var f = args.length > 0 ? args[0] : null;
	if (f != null && !AS3.isNum(f)) { a.sort(function(x, y) { return AS3.int(f(x, y)); }); return a; }
	var opt = f == null ? (args.length > 1 ? AS3.int(args[1]) : 0) : AS3.int(f);
	var desc = (opt & 2) != 0;
	var numeric = (opt & 16) != 0;
	var nocase = (opt & 1) != 0;
	a.sort(function(x, y) {
		var c = 0;
		if (numeric) { var p = AS3.num(x); var q = AS3.num(y); c = p < q ? -1 : (p > q ? 1 : 0); }
		else { var p = AS3.str(x); var q = AS3.str(y); if (nocase) { p = p.toLowerCase(); q = q.toLowerCase(); } c = p < q ? -1 : (p > q ? 1 : 0); }
		return desc ? -c : c;
	});
	return a;
};
AS3.sortOn = function(a, args) {
	var field = args.length > 0 ? AS3.str(args[0]) : "";
	var opt = args.length > 1 ? AS3.int(args[1]) : 0;
	var desc = (opt & 2) != 0;
	var numeric = (opt & 16) != 0;
	a.sort(function(x, y) {
		var p = AS3.at(x, field);
		var q = AS3.at(y, field);
		var c = 0;
		if (numeric) { var m = AS3.num(p); var n = AS3.num(q); c = m < n ? -1 : (m > n ? 1 : 0); }
		else { var m = AS3.str(p); var n = AS3.str(q); c = m < n ? -1 : (m > n ? 1 : 0); }
		return desc ? -c : c;
	});
	return a;
};

// a control's state, from a plain record or a class that reads it through a getter
AS3.control = function(c, k) {
	if (c == null) return null;
	switch (k) {
		case "UP": return c.get_UP != null ? c.get_UP() : c.UP;
		case "DOWN": return c.get_DOWN != null ? c.get_DOWN() : c.DOWN;
		case "LEFT": return c.get_LEFT != null ? c.get_LEFT() : c.LEFT;
		case "RIGHT": return c.get_RIGHT != null ? c.get_RIGHT() : c.RIGHT;
		default: return null;
	}
};

// ---- types ----------------------------------------------------------------------------------------

// a converted object carries the names of its class and the classes it extends
AS3.is = function(v, t) {
	if (v == null) return false;
	switch (t) {
		case "Object", "*": return true;
		case "String": return Std.isOfType(v, String);
		case "Number": return Std.isOfType(v, Float);
		case "int", "uint": return Std.isOfType(v, Float) && v == Std.int(v);
		case "Boolean": return v == true || v == false;
		case "Array": return !Std.isOfType(v, String) && v.push != null;
		default:
			var names = v.__is;
			return names != null && names.indexOf(t) >= 0;
	}
};

AS3.as = function(v, t) { return AS3.is(v, t) ? v : null; };

AS3.typeOf = function(v) {
	if (v == null) return "object";
	if (v == true || v == false) return "boolean";
	if (Std.isOfType(v, String)) return "string";
	if (Std.isOfType(v, Float)) return "number";
	return "object";
};
