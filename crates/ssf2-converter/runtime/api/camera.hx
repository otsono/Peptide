// The camera as the source's code sees it (PeptideAPI.getCamera()). Targets arrive as objects or as
// their clips; the engine's camera takes its own objects. Times are source frames.

// class PeptideCamera extends PeptideBaseAPIObject
var PeptideCamera = { name: "PeptideCamera" };
PeptideCamera.build = function(me) {
	PeptideBaseAPIObject.build(me);
	me.__is.push("PeptideCamera");
	me.__params = new haxe.ds.StringMap();
	me.__points = [];
	me.__type = "SSF2Camera";
	// its clip, as the source reads it: the view's centre and size, in the source's units
	me.__mc = { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
// the view as the source's camera clip shows it (the engine reports its centre and its size in
// the world)
PeptideCamera.refresh = function(me) {
	var c = match.getCamera();
	var s = PeptideRuntime.data.units.size;
	me.__mc.x = c.getX() / s;
	me.__mc.y = c.getY() / s;
	me.__mc.width = c.getViewportWidth() / s;
	me.__mc.height = c.getViewportHeight() / s;
};
	PeptideCamera.refresh(me);
};
PeptideCamera.addTarget = function(me, o) { var e = Stage.engineOf(o); if (e != null) match.getCamera().addTarget(e); };
PeptideCamera.deleteTarget = function(me, o) { var e = Stage.engineOf(o); if (e != null) match.getCamera().deleteTarget(e); };
PeptideCamera.addForcedTarget = function(me, o) { var e = Stage.engineOf(o); if (e != null) match.getCamera().addForcedTarget(e); };
PeptideCamera.deleteForcedTarget = function(me, o) { var e = Stage.engineOf(o); if (e != null) match.getCamera().deleteForcedTarget(e); };
PeptideCamera.addTimedTarget = function(me, o, n) { var e = Stage.engineOf(o); if (e != null) match.getCamera().addTimedTarget(e, AS3.num(n) * 2); };
PeptideCamera.deleteTimedCameraTarget = function(me, o) { var e = Stage.engineOf(o); if (e != null) match.getCamera().deleteTimedTarget(e); };
PeptideCamera.addTimedTargetPoint = function(me, p, n) {
	var s = PeptideRuntime.data.units.size;
	var q = EnginePoint.create(AS3.num(p.x) * s, AS3.num(p.y) * s);
	me.__points.push({ p: p, q: q });
	match.getCamera().addTimedTargetPoint(q, AS3.num(n) * 2);
};
PeptideCamera.deleteTimedTargetPoint = function(me, p) {
	for (r in me.__points) if (r.p == p) { match.getCamera().deleteTimedTargetPoint(r.q); me.__points.remove(r); break; }
};
// kept for the source to read back; this engine frames the shot itself
PeptideCamera.updateCameraParameters = function(me, o) {
	for (k in AS3.keysOf(o)) me.__params.set(AS3.str(k), AS3.at(o, k));
	if (o != null && o.minZoomHeight != null) me.__params.set("minZoomHeight", o.minZoomHeight);
};
// the stage dimmed behind the fighters (a final smash's): a filter on each of the camera's
// background layers, taken off again when it lightens
PeptideCamera.dimmed = null;
PeptideCamera.darken = function(me, on) {
	var layers = match.getCamera().getBackgroundContainers();
	if (PeptideCamera.dimmed != null) {
		for (r in PeptideCamera.dimmed) if (!r.c.isDisposed()) r.c.removeFilter(r.f);
		PeptideCamera.dimmed = null;
	}
	if (!on || layers == null) return;
	PeptideCamera.dimmed = [];
	for (c in layers) {
		var f = new HsbcColorFilter();
		f.brightness = -0.5;
		c.addFilter(f);
		PeptideCamera.dimmed.push({ c: c, f: f });
	}
};
PeptideCamera.init = function(me, ?api) {};
