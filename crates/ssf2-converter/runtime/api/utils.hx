// The utility helpers content calls (PeptideUtils.*): angles, speeds, distances, clip safety and colour,
// each answering what content expects of it. `cast_` hands back the object it is given (every
// object here is already the one content holds); colour goes through Visuals.

// class PeptideUtils
var PeptideUtils = { name: "PeptideUtils" };
PeptideUtils.toRadians = function(a) { return AS3.num(a) * (Math.PI / 180); };
PeptideUtils.toDegrees = function(a) { return AS3.num(a) * (180 / Math.PI); };
PeptideUtils.getDistance = function(a, b) {
	if (a == null || b == null) return 0;
	return Math.sqrt(Math.pow(a.x - b.x, 2) + Math.pow(a.y - b.y, 2));
};
PeptideUtils.calculateXSpeed = function(speed, angle) { return AS3.num(speed) * Math.cos(AS3.num(angle) * Math.PI / 180); };
PeptideUtils.calculateYSpeed = function(speed, angle) { return AS3.num(speed) * Math.sin(AS3.num(angle) * Math.PI / 180); };
PeptideUtils.calculateSpeed = function(x, y) { return Math.sqrt(Math.pow(AS3.num(x), 2) + Math.pow(AS3.num(y), 2)); };
PeptideUtils.forceBase360 = function(a) {
	a = AS3.num(a);
	while (a < 0) a += 360;
	while (a >= 360) a -= 360;
	return a;
};
PeptideUtils.calculateDifferenceBetweenAngles = function(a, b) {
	var d = AS3.num(b) - AS3.num(a);
	while (d < -180) d += 360;
	while (d > 180) d -= 360;
	return d;
};
PeptideUtils.getVelocityVector = function(speed, angle) {
	var p = Point.create();
	p.x = AS3.num(speed) * Math.cos(AS3.num(angle) * Math.PI / 180);
	p.y = AS3.num(speed) * Math.sin(AS3.num(angle) * Math.PI / 180);
	return p;
};
PeptideUtils.getAngleBetween = function(a, b) {
	return PeptideUtils.forceBase360(Math.atan2(-(b.y - a.y), b.x - a.x) * 180 / Math.PI);
};
// a speed eased toward rest: a non-negative rate scales it, a negative one takes that much off its
// size; it never crosses zero, and under half a pixel it is at rest
PeptideUtils.decel = function(v, d) {
	var speed = AS3.num(v);
	var rate = AS3.num(d);
	var next = rate >= 0 ? speed * rate : (speed > 0 ? Math.max(0, speed + rate) : Math.min(0, speed - rate));
	return Math.abs(next) < 0.5 ? 0 : next;
};
// the way the stick points, in degrees counterclockwise from right: one of eight, or -1 when it is
// centred or held two opposite ways at once
PeptideUtils.getControlsAngle = function(c) {
	var held = function(k) { return AS3.truthy(AS3.control(c, k)) ? 1 : 0; };
	var l = held("LEFT"); var r = held("RIGHT"); var u = held("UP"); var d = held("DOWN");
	if ((l == 1 && r == 1) || (u == 1 && d == 1)) return -1;
	var dx = r - l;
	var dy = u - d;
	if (dx == 0 && dy == 0) return -1;
	return PeptideUtils.forceBase360(Math.round(Math.atan2(dy, dx) * 180 / Math.PI));
};
PeptideUtils.safeGotoAndStop = function(mc, f) {
	if (mc == null) return;
	if (Std.isOfType(f, Float)) { if (f <= mc.totalFrames) mc.gotoAndStop(f); return; }
	if (MovieClip.frameOf(mc, f) > 0) mc.gotoAndStop(f);
};
PeptideUtils.removeFrameScripts = function(mc) { if (mc != null && mc.__scripts != null) mc.__scripts = new haxe.ds.IntMap(); };
PeptideUtils.safeRemoveMC = function(mc) { if (mc != null && mc.parent != null) mc.parent.removeChild(mc); };
// the box that holds a rectangle turned `deg` degrees counterclockwise about the origin (screen
// y runs down, so the turn is the other way round in its numbers)
PeptideUtils.rotateRectangleAroundOrigin = function(r, deg) {
	var out = r.clone();
	if (deg == 0) return out;
	var t = PeptideUtils.toRadians(deg);
	var cs = Math.cos(t);
	var sn = Math.sin(t);
	var xs = [out.x, out.x + out.width];
	var ys = [out.y, out.y + out.height];
	var x0 = 1e18; var y0 = 1e18;
	var x1 = -1e18; var y1 = -1e18;
	for (x in xs) for (y in ys) {
		var tx = x * cs + y * sn;
		var ty = y * cs - x * sn;
		x0 = Math.min(x0, tx); y0 = Math.min(y0, ty); x1 = Math.max(x1, tx); y1 = Math.max(y1, ty);
	}
	out.x = x0; out.y = y0; out.width = x1 - x0; out.height = y1 - y0;
	return out;
};
PeptideUtils.cast_ = function(o, ?cls) { return o; };
// a copy all the way down (the source round-trips it through JSON): arrays and keyed objects
PeptideUtils.cloneObject = function(o) {
	if (o == null || Std.isOfType(o, String) || Std.isOfType(o, Float) || o == true || o == false) return o;
	if (AS3.isArray(o)) { var a = []; for (x in o) a.push(PeptideUtils.cloneObject(x)); return a; }
	var out = {};
	for (k in AS3.keysOf(o)) AS3.putKey(out, k, PeptideUtils.cloneObject(AS3.at(o, k)));
	return out;
};
// a colour setting with every field the source fills in when it is not given (fields read by
// name: a record here cannot be given a field under a name worked out as it runs)
PeptideUtils.getCostumeObject = function(o) {
	var v = function(x, d) { return x == null ? d : x; };
	if (o == null) o = {};
	return { hue: v(o.hue, 0), saturation: v(o.saturation, 0), brightness: v(o.brightness, 0), contrast: v(o.contrast, 0),
		redMultiplier: v(o.redMultiplier, 1), greenMultiplier: v(o.greenMultiplier, 1), blueMultiplier: v(o.blueMultiplier, 1),
		alphaMultiplier: v(o.alphaMultiplier, 1), redOffset: v(o.redOffset, 0), greenOffset: v(o.greenOffset, 0),
		blueOffset: v(o.blueOffset, 0), alphaOffset: v(o.alphaOffset, 0) };
};
PeptideUtils.setColorFilters = function(target, o) { Visuals.filters(target, o); };
PeptideUtils.setBrightness = function(target, v) {
	v = AS3.num(v);
	if (Math.abs(v) > 100) v = v > 0 ? 100 : -100;
	Visuals.tint(target, ColorTransform.create(1, 1, 1, 1, v * 2.55, v * 2.55, v * 2.55, 0));
};
PeptideUtils.setTint = function(target, rm, gm, bm, am, ro, go, bo, ao) {
	Visuals.tint(target, ColorTransform.create(rm, gm, bm, am, ro, go, bo, ao));
};
PeptideUtils.replacePalette = function(mc, ?data, ?a, ?b) { PeptideRuntime.gap("replacePalette: recolouring a clip by palette"); };
