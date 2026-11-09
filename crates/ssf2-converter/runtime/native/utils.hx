// The API's helpers (`SSF2Utils`) as plain functions: a native object's code that calls one gets
// its block (compile/native.rs), under the API's own name.

// ---- forceBase360
// an angle brought into 0 up to 360
function forceBase360(a) {
	while (a < 0) a += 360;
	while (a >= 360) a -= 360;
	return a;
}

// ---- calculateDifferenceBetweenAngles
// how far one angle is from another, the short way round (-180 up to 180)
function calculateDifferenceBetweenAngles(a, b) {
	var d = b - a;
	while (d < -180) d += 360;
	while (d > 180) d -= 360;
	return d;
}

// ---- getAngleBetween
// the angle from one point to another, up from the right, in 0 up to 360
function getAngleBetween(a, b) {
	var d = Math.atan2(-(b.y - a.y), b.x - a.x) * 180 / Math.PI;
	while (d < 0) d += 360;
	while (d >= 360) d -= 360;
	return d;
}

// ---- decel
// a speed eased toward rest: a non-negative rate scales it, a negative one takes that much off its
// size; it never crosses zero, and under half a pixel it is at rest
function decel(v, d) {
	var next = d >= 0 ? v * d : (v > 0 ? Math.max(0, v + d) : Math.min(0, v - d));
	return Math.abs(next) < 0.5 ? 0 : next;
}

// ---- getVelocityVector
// a speed along an angle, as its x and y
function getVelocityVector(speed, angle) {
	return { x: speed * Math.cos(angle * Math.PI / 180), y: speed * Math.sin(angle * Math.PI / 180) };
}

// ---- darkenStage
// the stage behind the fighters dimmed (true), or back as it was (false). a look only: nothing
// a rollback puts back
var stageDimmed = [];
function darkenStage(on) {
	for (r in stageDimmed) if (!r.c.isDisposed()) r.c.removeFilter(r.f);
	stageDimmed = [];
	var layers = match.getCamera().getBackgroundContainers();
	if (!on || layers == null) return;
	for (c in layers) {
		var f = new HsbcColorFilter();
		f.brightness = -0.5;
		c.addFilter(f);
		stageDimmed.push({ c: c, f: f });
	}
}

// ---- stanceLabelOf
// the label another object's clip is on, as its own script publishes it (null for none)
function stanceLabelOf(o) {
	if (o == null || o.exports.__stanceLabel == null) return null;
	return o.exports.__stanceLabel();
}

// ---- platformBetween
// the stage's structure the line from `a` to `b` (in the source's units) crosses, the first one
// it meets of the stage's; one that can be dropped through is passed over where the options ask
// (`ignoreFallthrough`)
function platformBetween(a, b, ?options) {
	var solidOnly = options != null && options.ignoreFallthrough == true;
	if (a == null || b == null) return null;
	var from = Point.create(a.x * SIZE, a.y * SIZE);
	var to = Point.create(b.x * SIZE, b.y * SIZE);
	for (st in match.getStructures()) {
		if (solidOnly && st.getStructureStat("dropThrough") == true) continue;
		var pts = st.getAllPoints();
		if (pts == null) continue;
		var i = 0;
		while (i + 1 < pts.length) {
			if (Point.lineSegmentsIntersect(from, to, pts[i], pts[i + 1]) != null) return st;
			i++;
		}
	}
	return null;
}

// ---- labelOf
// the label an effect it holds is on: the animation it plays (each label of an effect is one)
function labelOf(effect) {
	return effect == null || effect.isDisposed() ? null : effect.getAnimation();
}

// ---- textSlice
// a part of a text, from `start` up to `end` (the rest, where it gives none), as Flash's
// `slice` cuts it: an index below 0 counts from the end
function textSlice(text, start, ?end) {
	var n = text.length;
	var a = start < 0 ? Std.int(Math.max(0, n + start)) : Std.int(Math.min(start, n));
	var b = end == null ? n : (end < 0 ? Std.int(Math.max(0, n + end)) : Std.int(Math.min(end, n)));
	return b > a ? text.substring(a, b) : "";
}

// ---- numberIn
// a whole number written in a base (2 to 36), as Flash's `toString(base)` writes it
function numberIn(n, base) {
	var digits = "0123456789abcdefghijklmnopqrstuvwxyz";
	var v = Std.int(n);
	if (v == 0) return "0";
	var neg = v < 0;
	if (neg) v = -v;
	var out = "";
	while (v > 0) {
		out = digits.charAt(v % base) + out;
		v = Std.int(v / base);
	}
	return neg ? "-" + out : out;
}

// ---- colorFilters
// the colour settings its code puts on an object (`setColorFilters`): its alpha, a hue/saturation/
// brightness/contrast filter, and a colour shader for the channels, each replacing the last it set
// on that object; none (`null`, `false`) takes them off
var colorFiltersOn = self.makeObject(null);
function colorFilters(t, o) {
	if (t == null || t.isDisposed()) return;
	if (colorFiltersOn.get() == null) colorFiltersOn.set(new haxe.ds.StringMap());
	var key = Std.string(t.getUid());
	var was = colorFiltersOn.get().get(key);
	if (was != null) {
		if (was.f != null) t.removeFilter(was.f);
		if (was.s != null) t.removeShader(was.s);
		colorFiltersOn.get().remove(key);
	}
	if (o == null || o == false) { t.setAlpha(1); return; }
	if (o.alphaMultiplier != null) t.setAlpha(o.alphaMultiplier);
	var on = { f: null, s: null };
	if (o.hue != null || o.saturation != null || o.brightness != null || o.contrast != null) {
		var h = new HsbcColorFilter();
		if (o.hue != null) h.hue = o.hue;
		if (o.saturation != null) h.saturation = o.saturation / 100;
		if (o.brightness != null) h.brightness = o.brightness / 100;
		if (o.contrast != null) h.contrast = o.contrast / 100;
		t.addFilter(h);
		on.f = h;
	}
	if (o.redMultiplier != null || o.greenMultiplier != null || o.blueMultiplier != null || o.redOffset != null || o.greenOffset != null || o.blueOffset != null) {
		var sh = new RgbaColorShader();
		sh.redMultiplier = o.redMultiplier != null ? o.redMultiplier : 1;
		sh.greenMultiplier = o.greenMultiplier != null ? o.greenMultiplier : 1;
		sh.blueMultiplier = o.blueMultiplier != null ? o.blueMultiplier : 1;
		sh.redOffset = o.redOffset != null ? o.redOffset : 0;
		sh.greenOffset = o.greenOffset != null ? o.greenOffset : 0;
		sh.blueOffset = o.blueOffset != null ? o.blueOffset : 0;
		t.addShader(sh);
		on.s = sh;
	}
	colorFiltersOn.get().set(key, on);
}

// ---- statOf
// what another object declares (`getItemStat`, `getEnemyStat`, `getProjectileStat` on it): what its
// script answers for its stats, else an item's name
function statOf(o, k) {
	if (o == null || o.exports == null) return null;
	if (o.exports.__statOf != null) return o.exports.__statOf(k);
	return k == "linkage_id" ? o.exports.__itemName : null;
}

// ---- takeDamageOf
// a hit its code deals another object outright (`takeDamage`), as the source's does: the damage,
// and the launch its stats state (base and growth against the damage it has and its weight, at the
// stated angle turned the way the one dealing it faces), with the freeze they state
function takeDamageOf(o, stats, ?by) {
	if (o == null || o.isDisposed() || stats == null) return true;
	var dmg = stats.damage == null ? 0 : stats.damage;
	var before = o.getDamage();
	if (dmg != 0) o.addDamage(dmg);
	var power = stats.power == null ? 0 : stats.power;
	// (a growth it does not state is the source's 100)
	var growth = stats.kbConstant == null ? 100 : stats.kbConstant;
	if ((power > 0 || growth > 0) && stats.hasEffect != false) {
		var weight = o.getType() == EntityType.CHARACTER ? o.getCharacterStat("weight") : 100;
		var t = before + dmg;
		var kb = ((((t * 0.1 + dmg * t * 0.05) * 1.4 * (200 / (weight + 100))) + 18) * (growth * 0.01)) + power;
		var dir = stats.direction == null ? 0 : stats.direction;
		if (by != null && !by.isFacingRight()) dir = 180 - dir;
		var r = dir * Math.PI / 180;
		var v = kb * 0.213725 * SPEED;
		o.setXKnockback(Math.cos(r) * v);
		o.setYKnockback(-Math.sin(r) * v);
	}
	if (stats.hitStun != null && stats.hitStun > 0 && o.getType() == EntityType.CHARACTER) o.forceStartHitstop(Std.int(stats.hitStun * 2));
	return true;
}

// ---- boundsOf
// a box of the stage's (its camera's or its death bounds), in the source's units
function boundsOf(a) {
	if (a == null) return null;
	var r = a.getRectangle();
	if (r == null) return null;
	var x = (a.getX() + r.x) / SIZE;
	var y = (a.getY() + r.y) / SIZE;
	var w = r.width / SIZE;
	var h = r.height / SIZE;
	return { x: x, y: y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h };
}

// ---- exportedStats
// an object's stats as a record to make it again by (`exportStats`): its name, the name it was
// fired or made under, and the fighter whose it is
function exportedStats(o) {
	if (o == null || o.exports == null) return null;
	var name = o.exports.__statOf != null ? o.exports.__statOf("linkage_id") : o.exports.__itemName;
	return { linkage_id: name, name: o.exports.__shotName, nme: o.exports.__itemName, chr: o.getOwner() };
}

// ---- fireStats
// a projectile made again from the record its stats were exported to: fired by the fighter whose
// it is, through the power it publishes, for this one
function fireStats(rec, ?x, ?y) {
	if (rec == null || rec.chr == null || rec.chr.exports == null) return null;
	var p = rec.chr.exports.__kirbyPower;
	return p == null || p.fire == null ? null : p.fire(rec.name, self, x, y);
}

// ---- itemsInMatch
// the match's items: every custom game object carrying the item handler
function itemsInMatch() {
	var out = [];
	for (o in match.getCustomGameObjects()) if (o.exports != null && o.exports.__isItem == true && !o.isDisposed()) out.push(o);
	return out;
}

// ---- pointDistance
// how far apart two points are
function pointDistance(a, b) {
	var dx = a.x - b.x;
	var dy = a.y - b.y;
	return Math.sqrt(dx * dx + dy * dy);
}

// ---- itemStatsOn
// stats written on another object that is an item: the engine's own where it keeps one (in this
// engine's units), and whether it can be picked up, through what its handler publishes
function itemStatsOn(o, stats) {
	if (o == null || o.isDisposed() || stats == null) return;
	var set = {};
	var any = false;
	if (stats.ghost != null) { set.ghost = stats.ghost; any = true; }
	if (stats.gravity != null) { set.gravity = stats.gravity * ACCEL; any = true; }
	if (stats.max_gravity != null) { set.terminalVelocity = stats.max_gravity * SPEED; any = true; }
	if (any && o.updateGameObjectStats != null) o.updateGameObjectStats(set);
	if (stats.canPickup != null && o.exports != null && o.exports.__ask != null) o.exports.__ask("pickup", stats.canPickup);
}
