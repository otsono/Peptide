// The API's helpers (`SSF2Utils`) as plain functions: a native object's code that calls one gets
// its block (compile/native.rs), under the API's own name.

// ---- decel
// a speed eased toward rest: a non-negative rate scales it, a negative one takes that much off its
// size; it never crosses zero, and under half a pixel it is at rest
function decel(v, d) {
	var next = d >= 0 ? v * d : (v > 0 ? Math.max(0, v + d) : Math.min(0, v - d));
	return Math.abs(next) < 0.5 ? 0 : next;
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

// ---- declaredOf
// what another object declares (`getItemStat`, `getEnemyStat`, `getProjectileStat` on it): its
// stats' metadata, which holds its declaration (read by the stat's own name: `declaredOf(o).damage`).
// a disposed object still reads its own; an object that declares nothing (a stock fighter) reads null
function declaredOf(o) {
	var d = o == null ? null : o.getGameObjectStatsMetadata();
	return d == null ? {} : d;
}

// ---- gameObjectStat
// a stat of any object by the source's name (`getGameObjectStat`): whether it can be hit is whether
// it is a fighter, else what it declares (`declared`, read from its metadata), else the engine's own stat
function gameObjectStat(o, k, declared) {
	if (o == null) return null;
	if (k == "canReceiveKnockback" || k == "canReceiveHits") return o.getType() == EntityType.CHARACTER;
	return declared != null ? declared : o.getGameObjectStat(k);
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
	var name = o.exports.__itemName != null ? o.exports.__itemName : declaredOf(o).linkage_id;
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

// ---- grabBy
// held by the fighter with that id, or (with none of that id) by the fighter whose object asks: the
// asker itself, else its root owner. answers whether it is held now
function grabBy(o, uid) {
	if (o == null || o.isDisposed()) return false;
	var g = null;
	for (c in match.getCharacters()) if (c.getUid() == uid) g = c;
	if (g == null) g = self.getType() == EntityType.CHARACTER ? self : self.getRootOwner();
	if (g == null || g == o || g.getType() != EntityType.CHARACTER) return false;
	g.attemptGrab(o);
	var held = g.getAllGrabbedFoes();
	return held != null && held.indexOf(o) >= 0;
}

// ---- boundsRectOf
// the box an object takes up, in the source's units: the size it declares, else its body's, standing
// on its feet where it stands
function boundsRectOf(o) {
	if (o == null || o.isDisposed()) return new Rectangle(0, 0, 0, 0);
	var d = o.getGameObjectStatsMetadata();
	var w = d == null || d.width == null ? 0.0 : d.width;
	var h = d == null || d.height == null ? 0.0 : d.height;
	if (w <= 0 || h <= 0) {
		w = Math.abs(o.getEcbRightHipX() - o.getEcbLeftHipX()) / SIZE;
		h = Math.abs(o.getEcbFootY() - o.getEcbHeadY()) / SIZE;
	}
	var x = o.getX() / SIZE;
	var y = o.getY() / SIZE;
	return new Rectangle(x - w / 2, y - h, w, h);
}

// ---- sizeOf
// how wide or tall an object is, in the source's units: the size it declares, else its body's
function sizeOf(o, tall) {
	if (o == null || o.isDisposed()) return 0;
	var d = o.getGameObjectStatsMetadata();
	var v = d == null ? null : (tall ? d.height : d.width);
	if (v != null && v > 0) return v;
	return (tall ? Math.abs(o.getEcbFootY() - o.getEcbHeadY()) : Math.abs(o.getEcbRightHipX() - o.getEcbLeftHipX())) / SIZE;
}

// ---- knockbackOf
// a launch by the source's formula, in its units: the box's growth and base against the damage
// dealt, the damage the one launched had and its weight (a charged one 1.2 times)
function knockbackOf(growth, base, fixed, damage, had, weight, ?charged) {
	var t = had + damage;
	var w = weight == null ? 100 : weight;
	var kb = ((((t * 0.1 + damage * t * 0.05) * 1.4 * (200 / (w + 100))) + 18) * (growth * 0.01)) + base;
	return charged == true ? kb * 1.2 : kb;
}

// ---- playerNumbered
// a player by number (from 1), as the match's fighters stand in player order
function playerNumbered(n) {
	var ps = match.getCharacters();
	var i = Std.int(n) - 1;
	return ps == null || i < 0 || i >= ps.length ? null : ps[i];
}

// ---- stagePlane
// one of the stage's planes, by name
function stagePlane(n) { return { plane: n }; }

// ---- cameraPlanes
// its camera backgrounds, each as the plane it is
function cameraPlanes() {
	var out = [];
	var l = stage == null || stage.exports == null ? null : stage.exports.__cameraBgs;
	if (l != null) for (i in 0...l.length) out.push({ mc: planePart(l[i]) });
	return out;
}

// ---- enemyStats
// stats a stage object's code gives it (source names and units): the engine's own for each it keeps,
// and all of them on its metadata, where other objects read it
function enemyStats(o, s) {
	if (o == null || o.isDisposed() || s == null) return;
	var set = {};
	var any = false;
	if (s.gravity != null) { set.gravity = s.gravity * ACCEL; any = true; }
	if (s.max_ySpeed != null && s.max_ySpeed > 0) { set.terminalVelocity = s.max_ySpeed * SPEED; any = true; }
	if (s.surviveDeathBounds != null) { set.deathBoundsDestroy = s.surviveDeathBounds != true; any = true; }
	if (s.ghost != null) { set.ghost = s.ghost == true; any = true; }
	if (any) o.updateGameObjectStats(set);
	if (s.canReceiveKnockback == false) o.applyGlobalBodyStatus(BodyStatus.LAUNCH_ARMOR, -1);
	if (o.getGameObjectStatsMetadata() == null) o.updateGameObjectStats({ metadata: s });
	else o.updateGameObjectStatsMetadata(s);
}

// ---- nearestOf
// the nearest object of a kind ("character", "player", "item", "projectile", "enemy") to one
function nearestOf(o, type) {
	var list = (type == "character" || type == "player") ? match.getCharacters() : (type == "enemy" || type == "item") ? match.getCustomGameObjects() : match.getProjectiles();
	var best = null;
	var bd = 1.0e12;
	for (x in list) {
		if (x == o || x.isDisposed()) continue;
		if (type == "item" && (x.exports == null || x.exports.__isItem != true)) continue;
		var dx = x.getX() - o.getX();
		var dy = x.getY() - o.getY();
		var d = dx * dx + dy * dy;
		if (d < bd) { bd = d; best = x; }
	}
	return best;
}

// ---- metadataWrite
// fields written onto an object's stats' metadata: merged where it declares some, set where it
// declares none (the merge has nothing to merge into)
function metadataWrite(o, s) {
	if (o == null || o.isDisposed() || s == null) return;
	if (o.getGameObjectStatsMetadata() == null) o.updateGameObjectStats({ metadata: s });
	else o.updateGameObjectStatsMetadata(s);
}

// ---- planePart
// a named clip of the stage's planes, as the stage publishes it (`exports.__part`)
function planePart(n) {
	return stage == null || stage.exports == null || stage.exports.__part == null ? null : stage.exports.__part(n);
}

// ---- sharedVarsOf
// what an object's code keeps by name (`setSharedVar`): its source's, where the runtime plays it, else
// what it publishes
function sharedVarsOf(o) {
	if (o == null || o.exports == null) return new haxe.ds.StringMap();
	var ex = o.exports;
	if (ex.__source != null && ex.__source.__vars != null) return ex.__source.__vars;
	if (ex.__vars == null) ex.__vars = new haxe.ds.StringMap();
	return ex.__vars;
}

// ---- foeAllowAttack
// another fighter's attack turned off or back on by the source's name: the actions the animations of
// that name start (each animation's prefix, and the action it is), each a status effect it holds
// (kept on what it publishes, `__actionsOff`)
function foeAllowAttack(o, on, anims) {
	if (o == null || o.exports == null) return;
	if (o.exports.__actionsOff == null) o.exports.__actionsOff = [];
	var off = o.exports.__actionsOff;
	var table = [
		["jab", CharacterActions.JAB], ["dash_attack", CharacterActions.DASH_ATTACK],
		["tilt_forward", CharacterActions.TILT_FORWARD], ["tilt_up", CharacterActions.TILT_UP], ["tilt_down", CharacterActions.TILT_DOWN],
		["strong_forward", CharacterActions.STRONG_FORWARD], ["strong_up", CharacterActions.STRONG_UP], ["strong_down", CharacterActions.STRONG_DOWN],
		["aerial_neutral", CharacterActions.AERIAL_NEUTRAL], ["aerial_forward", CharacterActions.AERIAL_FORWARD], ["aerial_back", CharacterActions.AERIAL_BACK],
		["aerial_up", CharacterActions.AERIAL_UP], ["aerial_down", CharacterActions.AERIAL_DOWN],
		["special_neutral", CharacterActions.SPECIAL_NEUTRAL], ["special_side", CharacterActions.SPECIAL_SIDE], ["special_up", CharacterActions.SPECIAL_UP], ["special_down", CharacterActions.SPECIAL_DOWN],
		["grab", CharacterActions.GRAB]
	];
	for (anim in anims) {
		var best = null;
		for (row in table) if (anim.indexOf(row[0]) == 0 && (best == null || row[0].length > best[0].length)) best = row;
		if (best == null) continue;
		var k = -1;
		for (i in 0...off.length) if (off[i].a == best[1]) k = i;
		if (on && k >= 0) { o.removeStatusEffect(StatusEffectType.DISABLE_ACTION, off[k].id); off.splice(k, 1); }
		else if (!on && k < 0) { var e = o.addStatusEffect(StatusEffectType.DISABLE_ACTION, best[1]); off.push({ a: best[1], id: e == null ? null : e.id }); }
	}
}

// ---- stageBounds
// the stage's camera and death bounds as its code keeps them (the source's units): read once from
// the engine's, then what its code writes. a write moves the engine's own: the death bounds by where
// they start, the camera held on the middle of the rectangle (`CameraMode.LOCKED`). their size is the
// engine's, which its code cannot change
var stageBoundsCam = null;
var stageBoundsDeath = null;
var stageViewHeight = 0.0;
function stageBoundsRec(camera) {
	var r = camera ? stageBoundsCam : stageBoundsDeath;
	if (r != null) return r;
	var a = camera ? stage.getCameraBounds() : stage.getDeathBounds();
	if (a == null) return null;
	var rr = a.getRectangle();
	if (rr == null) return null;
	var x = (a.getX() + rr.x) / SIZE;
	var y = (a.getY() + rr.y) / SIZE;
	r = { x: x, y: y, width: rr.width / SIZE, height: rr.height / SIZE, left: x, top: y, right: x + rr.width / SIZE, bottom: y + rr.height / SIZE };
	if (camera) stageBoundsCam = r; else stageBoundsDeath = r;
	return r;
}
function stageBoundsSet(camera, f, v) {
	var r = stageBoundsRec(camera);
	if (r == null || v == null || v != v) return;
	switch (f) {
		case "x": r.x = v;
		case "y": r.y = v;
		case "width": r.width = v;
		case "height": r.height = v;
	}
	r.left = r.x; r.top = r.y; r.right = r.x + r.width; r.bottom = r.y + r.height;
	if (f != "x" && f != "y") return;
	if (camera) {
		var c = match.getCamera();
		if (c == null) return;
		if (c.getMode() != CameraMode.LOCKED) c.setMode(CameraMode.LOCKED);
		// (showing the rectangle's height: the view's height at no zoom over it)
		// (the view's height unzoomed, read before the first zoom: the engine reports it zoomed after)
		if (stageViewHeight <= 0) stageViewHeight = c.getViewportHeight() / (c.getZoomScaleY() > 0 ? c.getZoomScaleY() : 1);
		if (r.height > 0) { var z = stageViewHeight / (r.height * SIZE); c.setLockedZoom(z, z); }
		c.setLockedPosition((r.x + r.width / 2) * SIZE, (r.y + r.height / 2) * SIZE);
	} else {
		var a = stage.getDeathBounds();
		if (a == null) return;
		var rr = a.getRectangle();
		a.updateCollisionAreaStats({ startX: r.x * SIZE - rr.x, startY: r.y * SIZE - rr.y });
	}
}

// ---- cameraParameter
// a setting of the stage's camera by the source's name: what the conversion declares it as (its
// cases: each setting and its value)
function cameraParameter(k) {
	switch (k) {
		// {cases}
	}
	return null;
}

// ---- madeObject
// an object the stage's code made (not counted with what its boot made, as the runtime counts)
function madeObject(id, o) {
	return o;
}

// ---- cameraBgName
// the name of one of the stage's camera backgrounds, by its place in its list (the stage publishes them)
function cameraBgName(i) {
	var l = stage == null || stage.exports == null ? null : stage.exports.__cameraBgs;
	return l == null || i < 0 || i >= l.length ? null : l[i];
}

// ---- safeGotoAndStop
// a clip sent to a frame or a label and held there, where it has one (a clip of the stage's planes)
function safeGotoAndStop(c, f) {
	if (c != null && c.playLabel != null) c.playLabel(f, true);
}

// ---- clipRecord
// a clip its code makes and moves (a raindrop, an ember), as a record: where it is, how it is
// drawn, the clips it holds, and the particle effect it is drawn as (`id`, with its size)
function clipRecord(id, ?w, ?h) {
	return { x: 0.0, y: 0.0, scaleX: 1.0, scaleY: 1.0, alpha: 1.0, rotation: 0.0, visible: true, name: null,
		kids: [], id: id, width: w == null ? 0.0 : w, height: h == null ? 0.0 : h, drawn: null };
}

// ---- clipAdd
// a clip put in another (its code's `addChild`), and taken out
function clipAdd(c, k) {
	if (c != null && k != null && c.kids != null && c.kids.indexOf(k) < 0) c.kids.push(k);
	return k;
}
function clipRemove(c, k) {
	if (c != null && c.kids != null) c.kids.remove(k);
	return k;
}

// ---- floorAttach
// put on the floor just under it, where there is one
function floorAttach(o) {
	if (o == null || o.isDisposed()) return;
	var out = Point.create(o.getX(), o.getY() + 4);
	var hits = o.hitTestStructuresWithLineSegment(Point.create(o.getX(), o.getY() - 4), out, out, { structureType: StructureType.FLOOR });
	if (hits == null || hits.length == 0) return;
	o.setY(out.y);
	o.attachToFloor(hits[0]);
}
