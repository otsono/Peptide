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

// ---- groundAt
// the stage's ground at a point (in the source's units): a structure whose line passes within a
// pixel of it, or the solid ground it is inside of (the structures' lines a line from it out to
// the right crosses an odd number of times: a platform's flat line is never crossed, so only an
// enclosed body holds a point). the options leave out what the source's do: platforms, terrain
// (`platforms`/`terrains: false`, `ignoreFallthrough`) or the structures listed (`ignoreList`)
function groundAt(x, y, ?o) {
	var platforms = o == null || (o.platforms != false && o.ignoreFallthrough != true);
	var terrains = o == null || (o.terrains != false && o.terrain != false);
	var ignore = o == null ? null : o.ignoreList;
	var near = [Point.create(x * SIZE, (y - 1) * SIZE), Point.create(x * SIZE, (y + 1) * SIZE)];
	var from = Point.create(x * SIZE, y * SIZE);
	var to = Point.create(x * SIZE + 100000, y * SIZE);
	var crossed = 0;
	var last = null;
	for (st in match.getStructures()) {
		var drop = st.getStructureStat("dropThrough") == true;
		if (drop ? !platforms : !terrains) continue;
		if (ignore != null && ignore.indexOf(st) >= 0) continue;
		var pts = st.getAllPoints();
		if (pts == null) continue;
		var i = 0;
		while (i + 1 < pts.length) {
			if (Point.lineSegmentsIntersect(near[0], near[1], pts[i], pts[i + 1]) != null) return st;
			if (Point.lineSegmentsIntersect(from, to, pts[i], pts[i + 1]) != null) { crossed++; last = st; }
			i++;
		}
	}
	return crossed % 2 == 1 ? last : null;
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
// (a text joined to another, or a list to another list)
function anyConcat(v, x) {
	if (v == null) return null;
	if (Std.isOfType(v, String)) return v + (x == null ? "null" : Std.string(x));
	return v.concat(x);
}
// (a value of no known kind: a text cut the same way, a list by its own slice)
function anySlice(v, start, ?end) {
	if (v == null) return null;
	if (Std.isOfType(v, String)) return textSlice(v, start, end);
	return end == null ? v.slice(start) : v.slice(start, end);
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
// (the colour settings its code last put on an object, which put back are the same)
function colorFiltersOf(t) {
	if (t == null || t.getUid == null || colorFiltersOn.get() == null) return null;
	var was = colorFiltersOn.get().get(Std.string(t.getUid()));
	return was == null ? null : was.o;
}
function colorFilters(t, o) {
	// (a copy of a fighter's body, `bodyCopy`: drawn in that fighter's own colours already)
	if (t != null && t.body != null) return;
	// (a library clip its code made, a record: its colours are drawn nowhere here)
	if (t == null || t.isDisposed == null || t.isDisposed()) return;
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
	var on = { f: null, s: null, o: o };
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

// ---- cameraClip
// the camera's clip, as the source reads it: the view's centre and size, in the stage's space and
// the source's units
function cameraClip() {
	var c = match.getCamera();
	return { x: c.getX() / SIZE + ORIGIN_X, y: c.getY() / SIZE + ORIGIN_Y, width: c.getViewportWidth() / SIZE, height: c.getViewportHeight() / SIZE };
}

// ---- screenToStage
// a point on the screen (the source's 640 by 360) in the stage's space: where it falls in the
// camera's view, its centre and size as the source reads them (`cameraClip`)
function screenToStage(p) {
	var c = cameraClip();
	var x = p == null ? 0.0 : p.x;
	var y = p == null ? 0.0 : p.y;
	return { x: c.x + (x / 640 - 0.5) * c.width, y: c.y + (y / 360 - 0.5) * c.height };
}

// ---- boundsOf
// a box of the stage's (its camera's or its death bounds), in the source's units and the space of
// the code asking (a stage's, from the source's origin)
function boundsOf(a) {
	if (a == null) return null;
	var r = a.getRectangle();
	if (r == null) return null;
	var x = (a.getX() + r.x) / SIZE + ORIGIN_X;
	var y = (a.getY() + r.y) / SIZE + ORIGIN_Y;
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
	// (asked by the stage, which no fighter is: a hold by nobody, which this engine has none of)
	if (g == null && self.getType == null) return false;
	if (g == null) g = self.getType() == EntityType.CHARACTER ? self : self.getRootOwner();
	if (g == null || g == o || g.getType() != EntityType.CHARACTER) return false;
	g.attemptGrab(o);
	var held = g.getAllGrabbedFoes();
	return held != null && held.indexOf(o) >= 0;
}

// ---- boundsRectOf
// the box an object takes up, in the source's units: the size it declares, else its body's, standing
// on its feet where it stands
function boundsRectOf(o, ?ox, ?oy) {
	if (o == null || o.isDisposed()) return new Rectangle(0, 0, 0, 0);
	var d = o.getGameObjectStatsMetadata();
	var w = d == null || d.width == null ? 0.0 : d.width;
	var h = d == null || d.height == null ? 0.0 : d.height;
	if (w <= 0 || h <= 0) {
		w = Math.abs(o.getEcbRightHipX() - o.getEcbLeftHipX()) / SIZE;
		h = Math.abs(o.getEcbFootY() - o.getEcbHeadY()) / SIZE;
	}
	var x = o.getX() / SIZE + (ox == null ? 0 : ox);
	var y = o.getY() / SIZE + (oy == null ? 0 : oy);
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
	// (a clip handed over itself: that clip)
	if (n != null && !Std.isOfType(n, String)) return n;
	return stage == null || stage.exports == null || stage.exports.__part == null ? null : stage.exports.__part(n);
}

// ---- fadeTo
// a fade its code steps once a frame, as it asks (a tenth of the way each source frame); fading while
// it has not got there
var fadingNow = self.makeObject(false);
function fadeTo(o, to) {
	var a = o.getAlpha();
	a = to > a ? Math.min(to, a + 0.1) : Math.max(to, a - 0.1);
	o.setAlpha(a);
	fadingNow.set(a != to);
}

// ---- clipJump
// a clip a record keeps sent to a frame or label and held or played on: a part of the planes the way
// it is sent, an effect by its own frame calls; to nothing, held or played where it is
function clipJump(c, to, hold) {
	if (c == null) return;
	// (a clip record with a timeline of its own, `clipRun`: its timeline's)
	if (c.labelList != null && c.totalFrames != null) { clipGo(c, to, hold); return; }
	if (c.playLabel != null) { if (to == null) c.hold(hold); else c.playLabel(to, hold); return; }
	if (to != null) { if (Std.isOfType(to, String)) c.playFrameLabel(to); else c.playFrame(Std.int(to) * 2 - 1); }
	if (hold) c.pause(); else c.resume();
}

// ---- stageLedges
// the ledges of the stage's floors, in the stage's space: each where its floor ends, as a clip record
function stageLedges() {
	var out = [];
	for (st in match.getStructures()) {
		if (st.getAllPoints == null) continue;
		var pts = st.getAllPoints();
		if (pts == null || pts.length < 2) continue;
		if (st.getStructureStat("leftLedge") == true) out.push({ x: pts[0].x / SIZE + ORIGIN_X, y: pts[0].y / SIZE + ORIGIN_Y, visible: true });
		if (st.getStructureStat("rightLedge") == true) { var p = pts[pts.length - 1]; out.push({ x: p.x / SIZE + ORIGIN_X, y: p.y / SIZE + ORIGIN_Y, visible: true }); }
	}
	return out;
}

// ---- qualityPlays
// the clips the stage's quality settings animate at the highest quality: played on every frame,
// whatever their own code stops
var playing = self.makeObject(null);
function qualityPlays(l) {
	if (l == null) return;
	if (playing.get() == null) { playing.set([]); self.addTimer(1, -1, keepPlaying, { persistent: true }); }
	for (c in l) if (c != null) playing.get().push(c);
}
function keepPlaying() {
	var l = playing.get();
	if (l == null) return;
	for (c in l) if (c.hold != null) c.hold(false); else if (c.resume != null) c.resume();
}

// ---- stagePlace
// where an object is in the stage's space (ORIGIN_X, ORIGIN_Y: the source's origin in this engine's,
// in the source's units), as the source has it: from where its clip is registered, which an object
// of the stage publishes (`exports.__anchor`, this engine's units, from where it stands)
function anchorOf(o) { return o == null || o.exports == null || o.exports.__anchor == null ? null : o.exports.__anchor; }
function placeX(o) { if (o == null) return 0.0; var a = anchorOf(o); return (o.getX() - (a == null ? 0 : a.x)) / SIZE + ORIGIN_X; }
function placeY(o) { if (o == null) return 0.0; var a = anchorOf(o); return (o.getY() - (a == null ? 0 : a.y)) / SIZE + ORIGIN_Y; }
function setPlaceX(o, v) { if (o == null) return; var a = anchorOf(o); o.setX((v - ORIGIN_X) * SIZE + (a == null ? 0 : a.x)); }
function setPlaceY(o, v) { if (o == null) return; var a = anchorOf(o); o.setY((v - ORIGIN_Y) * SIZE + (a == null ? 0 : a.y)); }

// ---- setBrightness
// an object brightened or darkened by its code (-100 to 100, as the source's), replacing the last
// it set on that object
var brightOn = self.makeObject(null);
function setBrightness(t, v) {
	if (t == null || t.isDisposed()) return;
	if (v > 100) v = 100;
	if (v < -100) v = -100;
	if (brightOn.get() == null) brightOn.set(new haxe.ds.StringMap());
	var key = Std.string(t.getUid());
	var was = brightOn.get().get(key);
	if (was != null) t.removeShader(was);
	var sh = new RgbaColorShader();
	sh.redOffset = v * 2.55;
	sh.greenOffset = v * 2.55;
	sh.blueOffset = v * 2.55;
	t.addShader(sh);
	brightOn.get().set(key, sh);
}

// ---- startPosition
// where it was the first time its code asked (where it was put, before its own code moved it), in the
// source's units and space as its own `getX` reads
var startAt = self.makeObject(null);
function startPosition() {
	if (startAt.get() == null) {
		var a = self.exports == null ? null : self.exports.__anchor;
		startAt.set({ x: (self.getX() - (a == null ? 0 : a.x)) / SIZE + ORIGIN_X, y: (self.getY() - (a == null ? 0 : a.y)) / SIZE + ORIGIN_Y });
	}
	var p = startAt.get();
	return { x: p.x, y: p.y };
}

// ---- stageField
// a field of the stage's own class, as the stage publishes it for other objects' code
function stageField(n) {
	var c = stage == null || stage.exports == null || stage.exports.__fieldOf == null ? null : stage.exports.__fieldOf("self", n);
	return c == null ? null : c.get();
}
// and written
function setStageField(n, v) {
	var c = stage == null || stage.exports == null || stage.exports.__fieldOf == null ? null : stage.exports.__fieldOf("self", n);
	if (c != null) c.set(v);
	return v;
}

// ---- moveLegs
// a path it moves along, a leg at a time (`addMovement`): each leg moves it by its speeds (source
// pixels a frame) for its time, then holds it for its wait, and the last leads back to the first.
// stepped by a timer of its own every engine frame, half a source frame a step
var legs = self.makeObject(null);
var legAt = self.makeInt(0);
var legT = self.makeFloat(0);
function addMove(m) {
	if (legs.get() == null) { legs.set([]); self.addTimer(1, -1, stepLegs, { persistent: true }); }
	legs.get().push(m);
}
function stepLegs() {
	var all = legs.get();
	if (all == null || all.length == 0) return;
	var m = all[legAt.get()];
	var time = m.moveTime == null ? 0 : m.moveTime;
	var wait = m.waitTime == null ? 0 : m.waitTime;
	if (legT.get() < time) {
		if (m.xSpeed != null) self.setX(self.getX() + m.xSpeed * SIZE * 0.5);
		if (m.ySpeed != null) self.setY(self.getY() + m.ySpeed * SIZE * 0.5);
	}
	legT.set(legT.get() + 0.5);
	if (legT.get() >= time + wait) { legT.set(0); legAt.set((legAt.get() + 1) % all.length); }
}

// ---- homeTowards
// sent at a target, as the source's engine does it: the angle from where it is to where the target
// stands taken the source's way (its y term is the target's height negated, minus its own: not the
// difference), the speed along that angle, and a turn to face the way it goes. a target is an
// object, or a point in the source's units
function homeTowards(o, speed, t) {
	if (o == null || t == null) return;
	var tx = t.getX != null ? t.getX() / SIZE : t.x;
	var ty = t.getY != null ? t.getY() / SIZE : t.y;
	var a = Math.atan2(-ty - o.getY() / SIZE, tx - o.getX() / SIZE);
	var xs = speed * Math.cos(a);
	var ys = -speed * Math.sin(a);
	if (xs <= 0) o.faceLeft(); else o.faceRight();
	o.setXSpeed(Math.abs(xs) * SPEED);
	o.setYSpeed(ys * SPEED);
}

// ---- reversedList
// a list turned back to front in place, and handed back
function reversedList(l) {
	if (l == null) return l;
	l.reverse();
	return l;
}

// ---- joinedList
// a list's items as one text, between each the separator given (a comma where none is)
function joinedList(l, ?sep) { return l == null ? "" : l.join(sep == null ? "," : sep); }

// ---- gameSettings
// the match's settings as the source's code reads them: a versus match on none of its stages, none
// of its special modes on, and a player for each fighter
function gameSettings() {
	var players = [];
	for (c in match.getCharacters()) players.push({ name: "", team: c.getTeam(), costume: c.getCostumeIndex(), human: true });
	return { levelData: { stage: "", specialModes: 0 }, playerSettings: players };
}

// ---- keysIn
// the names an object holds, as a loop over them (`for (k in o)`): a list's places, a map's keys
function keysIn(o) {
	var out = [];
	if (o == null) return out;
	if (o.push != null) { for (i in 0...o.length) out.push(i); return out; }
	if (o.keys != null) for (k in o.keys()) out.push(k);
	return out;
}

// ---- gameVars
// what the match keeps by name for every object's code (`setGlobalVar`): on the stage, which all of
// them reach
var gameVarsHere = new haxe.ds.StringMap();
function gameVars() {
	var st = stage;
	if (st == null || st.exports == null) return gameVarsHere;
	if (st.exports.__gameVars == null) st.exports.__gameVars = new haxe.ds.StringMap();
	return st.exports.__gameVars;
}

// ---- teammatesOf
// the fighters on its team but itself
function teammatesOf(o) {
	var out = [];
	if (o == null) return out;
	for (c in match.getCharacters()) if (c != o && c.getTeam() == o.getTeam()) out.push(c);
	return out;
}

// ---- getDistance
// how far apart two points are
function getDistance(a, b) {
	if (a == null || b == null) return 0.0;
	var dx = b.x - a.x;
	var dy = b.y - a.y;
	return Math.sqrt(dx * dx + dy * dy);
}

// ---- objectsOf
// every object on the stage whose class is the one named (as each publishes the classes it is)
function objectsOf(kind) {
	var out = [];
	var all = match.getCustomGameObjects();
	if (all != null) for (o in all) if (o != null && o.exports != null && o.exports.__classes != null && o.exports.__classes.indexOf(kind) >= 0) out.push(o);
	return out;
}

// ---- floorOwner
// the object whose floor a fighter stands on (one of the faces a stage object's body makes), else
// none: the source's platform is an object, the engine's floors of the stage are not
function floorOwner(c) {
	if (c == null || c.getCurrentFloor == null) return null;
	var st = c.getCurrentFloor();
	if (st == null || stage == null || stage.exports == null || stage.exports.__faceOwners == null) return null;
	for (fo in stage.exports.__faceOwners) if (fo.st == st) return (fo.o != null && !fo.o.isDisposed()) ? fo.o : null;
	return null;
}

// ---- drawOrderParent
// the clip an object's clip is drawn in, as its code reads it for drawing order: this engine draws
// each object on its own layer, so every place in it is the first and moving one changes nothing
function drawOrderParent() {
	return { getChildIndex: function(c) { return 0; }, setChildIndex: function(c, i) { return null; }, numChildren: 0 };
}

// ---- replaceFirst
// a text with the first place a piece of text appears in it put in place of (the source's
// `replace` with a text to find, which changes only the first)
function replaceFirst(s, a, b) {
	if (s == null) return null;
	var t = Std.string(s);
	var i = t.indexOf(a);
	return i < 0 ? t : t.substr(0, i) + b + t.substr(i + a.length);
}

// ---- staticVar
// a class's static variable its code changes as it runs: one value for every object of the class,
// kept on the stage by the class's name and its own (its declared value until first written)
var staticsHere = self.makeObject(null);
function staticsOf() {
	if (stage != null && stage.exports != null) {
		if (stage.exports.__statics == null) stage.exports.__statics = new haxe.ds.StringMap();
		return stage.exports.__statics;
	}
	if (staticsHere.get() == null) staticsHere.set(new haxe.ds.StringMap());
	return staticsHere.get();
}
function staticVar(k, d) { var m = staticsOf(); return m.exists(k) ? m.get(k) : d; }
function setStaticVar(k, v) { staticsOf().set(k, v); return v; }

// ---- nextUid
// a fresh id each time it is asked: one count for the whole match, kept on the stage, so no two
// objects' ids meet (a rollback never hands an id out twice: it only skips some)
var uidAt = self.makeInt(0);
function nextUid() {
	if (stage == null || stage.exports == null) { uidAt.set(uidAt.get() + 1); return uidAt.get(); }
	if (stage.exports.__uid == null) stage.exports.__uid = 0;
	stage.exports.__uid = stage.exports.__uid + 1;
	return stage.exports.__uid;
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
		["special_neutral", CharacterActions.SPECIAL_NEUTRAL], ["special_side", CharacterActions.SPECIAL_SIDE], ["special_up", CharacterActions.SPECIAL_UP], ["special_down", CharacterActions.SPECIAL_DOWN]
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

// ---- pixels
// the pictures its code draws on and from (`BitmapData`), drawn with the engine's sprites: a picture
// is a record (`pxCanvas`): one of its library's (`anim`, the stage sprite's animation showing it),
// one placed on the stage (where its corner sits and its scale, this engine's units), or one its
// code makes (drawn nowhere until it is copied onto one that is). what its code draws on one is a
// sprite per place it draws at: a part of another picture (masked to the rectangle copied), or a
// colour (the white pixel, tinted and stretched over the rectangle filled). a place drawn at again
// is that sprite changed, as a picture keeps only the last paint there
var pxSprites = 0;
function pxCanvas(anim, w, h) {
	return { anim: anim, width: w == null ? 0 : w, height: h == null ? 0 : h, x: 0.0, y: 0.0, sx: 1.0, sy: 1.0, placed: false,
		root: null, paints: new haxe.ds.StringMap(), shader: null, cls: anim, rect: null };
}
// a picture shown on its own (`new Bitmap(data)`): drawn where its code puts it once it is in a plane
function pxBitmap(data) {
	return { kind: "bitmap", lib: true, bitmapData: data, x: 0.0, y: 0.0, scaleX: 1.0, scaleY: 1.0, alpha: 1.0, visible: true, part: null, plane: null, vs: [], parts: [] };
}
// one the tree placed (a plane's child, `getChildAt`)
function pxPlaced(k) {
	if (k == null) return null;
	if (k.canvas == null) {
		var c = pxCanvas(null, k.w, k.h);
		c.cls = k.b; c.x = k.x; c.y = k.y; c.sx = k.sx; c.sy = k.sy; c.placed = true;
		k.canvas = { bitmapData: c, x: k.x / SIZE, y: k.y / SIZE, width: k.w, height: k.h, kind: "bitmap" };
	}
	return k.canvas;
}
// the container its paints are drawn in: over the stage's art where the tree placed it
function pxRoot(c) {
	if (c.root != null) return c.root;
	c.root = Container.create();
	if (c.placed) {
		stage.getBackgroundEffectsContainer().addChild(c.root);
		c.root.x = c.x;
		c.root.y = c.y;
		c.root.scaleX = c.sx;
		c.root.scaleY = c.sy;
	}
	return c.root;
}
function pxSprite(anim) {
	var s = Sprite.create(self.getResource().getContent(STAGE_ID));
	s.currentAnimation = anim;
	pxSprites++;
	return s;
}
// a colour over a rectangle of it (`fillRect`, a pixel for `setPixel`); the clear colour of a
// picture its code made takes back what was drawn there
function pxFill(c, x, y, w, h, color) {
	if (c == null) return;
	var key = "f" + x + "," + y + "," + w + "," + h;
	var clear = !c.placed && (color >>> 24) == 0;
	if (clear && x <= 0 && y <= 0 && w >= c.width && h >= c.height) { pxClear(c); return; }
	var p = c.paints.get(key);
	if (clear) { if (p != null) p.s.visible = false; return; }
	if (p == null) {
		var sp = pxSprite("px_white");
		var sh = new SolidColorShader();
		sp.addShader(sh);
		p = { s: sp, sh: sh };
		c.paints.set(key, p);
	}
	p.sh.color = color | 0xff000000;
	p.s.x = x; p.s.y = y; p.s.scaleX = w; p.s.scaleY = h;
	if (!p.s.visible || !p.on) { p.s.visible = true; p.on = true; pxRoot(c).addChild(p.s); }
}
function pxClear(c) {
	// (hidden, kept for the next paint at the same place: a picture cleared and drawn again each
	// frame reuses its sprites)
	for (k in c.paints.keys()) { var p = c.paints.get(k); if (p.box != null) p.box.visible = false; else if (p.s != null) p.s.visible = false; }
}
// a rectangle of another picture copied onto it at a point (`copyPixels`): that picture's sprite,
// moved so the rectangle lands on the point and masked to it. a picture its code made is moved
// whole (it is drawn in one place at a time)
function pxCopy(c, src, r, pt) {
	if (c == null || src == null || r == null || pt == null) return;
	var key = "c" + pt.x + "," + pt.y + ":" + (src.anim == null ? "made" : src.anim);
	var p = c.paints.get(key);
	if (p == null) {
		var box = Container.create();
		var mask = pxSprite("px_white");
		box.addChild(mask);
		box.addFilter(new MaskFilter(mask));
		p = { box: box, mask: mask, s: null };
		c.paints.set(key, p);
	}
	if (src.anim != null) {
		if (p.s == null) { p.s = pxSprite(src.anim); p.box.addChild(p.s); }
		if (src.shader != null && p.sh != src.shader) { if (p.sh != null) p.s.removeShader(p.sh); p.s.addShader(src.shader); p.sh = src.shader; }
		p.s.x = -r.x; p.s.y = -r.y;
	} else {
		var made = pxRoot(src);
		if (p.s != made) { made.remove(); p.box.addChild(made); p.s = made; }
		made.x = -r.x; made.y = -r.y;
	}
	p.mask.x = 0; p.mask.y = 0; p.mask.scaleX = r.width; p.mask.scaleY = r.height;
	p.box.x = pt.x; p.box.y = pt.y;
	if (!p.box.visible || !p.on) { p.box.visible = true; p.on = true; pxRoot(c).addChild(p.box); }
}
// its colours changed (`colorTransform`): every sprite it shows takes them, and what it shows from
// then on (a number in their place changes nothing)
function pxTint(c, ct) {
	if (c == null || ct == null || !Std.isOfType(ct, RgbaColorShader)) return;
	c.shader = ct;
	for (k in c.paints.keys()) {
		var p = c.paints.get(k);
		if (p.s != null && p.box == null && p.sh != ct) { if (p.sh != null) p.s.removeShader(p.sh); p.s.addShader(ct); p.sh = ct; }
		else if (p.s != null && p.box != null && p.sh2 != ct) { if (p.sh2 != null) p.s.removeShader(p.sh2); p.s.addShader(ct); p.sh2 = ct; }
	}
}

// ---- followLight
// the stage's light where its code keeps it (over the fighters as they go). this engine's light
// cannot move as it runs, so the stage casts the fighters' shadows itself: each fighter's own frame,
// darkened, laid on the ground it stands over and slanted away from the light, as a light at that
// place casts it, inside the stage's shadow mask (its `shadowMask` animation). FOLLOW_LIGHT: where
// the light starts (this engine's units)
var followAt = FOLLOW_LIGHT;
// the stage's floors (FOLLOW_FLOORS: runs [x0, y0, x1, y1]): a shadow lands on the highest one under
// a fighter in the air
var followFloors = FOLLOW_FLOORS;
function followGround(x, y) {
	var best = null;
	for (f in followFloors) {
		var lo = Math.min(f[0], f[2]);
		var hi = Math.max(f[0], f[2]);
		if (x < lo || x > hi) continue;
		var fy = f[2] == f[0] ? Math.min(f[1], f[3]) : f[1] + (f[3] - f[1]) * (x - f[0]) / (f[2] - f[0]);
		if (fy >= y - 0.5 && (best == null || fy < best)) best = fy;
	}
	return best;
}
var followBox = null;
var followCasts = [];
// its code moves the light along the stage (the source's units)
function followLightAt(x) {
	if (followAt == null || x == null || x != x) return;
	followAt = [x * SIZE, followAt[1]];
}
function followShadows() {
	if (followAt == null) return;
	if (followBox == null) {
		followBox = Container.create();
		var mask = Sprite.create(self.getResource().getContent(STAGE_ID));
		mask.currentAnimation = "shadowMask";
		followBox.addChild(mask);
		followBox.addFilter(new MaskFilter(mask));
		stage.getCharactersBackContainer().addChild(followBox);
	}
	var chars = match.getCharacters();
	for (i in 0...chars.length) {
		var c = chars[i];
		if (followCasts.length <= i) {
			var sp = Sprite.create(c.getGameObjectStat("spriteContent"));
			var sh = new SolidColorShader();
			sh.color = 0xff000000;
			sp.addShader(sh);
			sp.alpha = 0.5;
			followBox.addChild(sp);
			followCasts.push({ s: sp, ground: c.getY() });
		}
		var shadow = followCasts[i];
		var sp = shadow.s;
		if (c == null || c.isDisposed() || !c.getSprite().visible) { sp.visible = false; continue; }
		// (the ground under it: where it stands, else the highest floor below it; none, and its shadow
		// is nowhere, as over a pit)
		if (c.isOnFloor()) shadow.ground = c.getY();
		else shadow.ground = followGround(c.getX(), c.getY());
		if (shadow.ground == null) { sp.visible = false; continue; }
		var fx = c.getX();
		var fy = c.getY();
		var g = shadow.ground;
		var lx = followAt[0];
		var ly = followAt[1];
		if (fy <= ly || g <= ly) { sp.visible = false; continue; }
		sp.visible = true;
		sp.currentAnimation = c.getAnimation();
		sp.currentFrame = c.getCurrentFrame();
		// its feet, carried along the ray from the light down to the ground; its height laid flat
		// along the ground, leaning away from the light
		var at = lx + (fx - lx) * (g - ly) / (fy - ly);
		var lean = -(fx - lx) / (g - ly);
		var face = c.isFacingRight() ? 1.0 : -1.0;
		sp.transformationMatrix = new Matrix(face, 0, lean, 0.25, at, g);
	}
}

// ---- ignoreOn
// a platform that lets one object through, or no longer does (`addIgnoreObject`): a structure's own
// blacklist, or the faces of an object that stands as a body (it publishes `__ignore`)
function ignoreOn(p, o, on) {
	if (p == null || o == null) return;
	var ex = null;
	try { ex = p.exports; } catch (e:Dynamic) {}
	// (a list turned round, `setIgnoreObjectListInversed`: only the listed touch it, so every fighter
	// but those is let through)
	var inv = ignoreInverted(p);
	if (inv != null) {
		inv.allow.remove(o);
		if (on) inv.allow.push(o);
		// (each let through once, and only one it let through taken back: the engine has no list entry
		// to take out otherwise)
		for (c in match.getCharacters()) {
			var block = inv.allow.indexOf(c) < 0;
			var was = inv.blocked.indexOf(c) >= 0;
			if (block && !was) { inv.blocked.push(c); ignoreOne(p, ex, c, true); }
			if (!block && was) { inv.blocked.remove(c); ignoreOne(p, ex, c, false); }
		}
		return;
	}
	ignoreOne(p, ex, o, on);
}
function ignoreOne(p, ex, o, on) {
	if (ex != null && ex.__ignore != null) { ex.__ignore(o, on); return; }
	if (on) p.addToBlacklist(o); else p.removeFromBlacklist(o);
}
var ignoreTurned = [];
function ignoreInverted(p) { for (r in ignoreTurned) if (r.p == p) return r; return null; }
function ignoreInverse(p, on) {
	if (p == null) return;
	var r = ignoreInverted(p);
	if (on == true && r == null) ignoreTurned.push({ p: p, allow: [], blocked: [] });
	if (on != true && r != null) ignoreTurned.remove(r);
}

// ---- libClips
// clips its code makes from the library (`getMCByLinkageName`) and puts in a plane: each a record its
// code moves as the source moves the clip, drawn every frame as the effects of its pieces where it
// is (LIB_ROWS: each clip's pieces, the effect and its corner)
var stageLibRows = LIB_ROWS;
var libsMade = [];
function libClip(n) {
	var row = null;
	for (r in stageLibRows) if (r.n == n) row = r;
	return { lib: true, n: n, parts: row == null ? [] : row.parts, x: 0.0, y: 0.0, scaleX: 1.0, scaleY: 1.0, alpha: 1.0, visible: true, part: null, plane: null, vs: [], name: n, currentFrame: 1 };
}
function libAdd(c, part, plane) {
	if (c == null || c.lib != true) return c;
	c.part = part;
	c.plane = plane;
	if (libsMade.indexOf(c) < 0) libsMade.push(c);
	return c;
}
function libRemove(c) {
	if (c == null || c.lib != true) return;
	if (c.kind == "bitmap") { if (c.bitmapData != null && c.bitmapData.root != null) c.bitmapData.root.remove(); c.into = null; libsMade.remove(c); return; }
	for (e in c.vs) e.v.destroy();
	c.vs = [];
	libsMade.remove(c);
	c.part = null;
}
// the container a plane's clips are drawn in: one of the camera's backgrounds where the plane is one
function libInto(plane) {
	var bgs = stage == null || stage.exports == null ? null : stage.exports.__cameraBgs;
	if (bgs != null) for (i in 0...bgs.length) if (bgs[i] == plane) {
		var cs = match.getCamera().getBackgroundContainers();
		if (cs != null && cs.length > i) return cs[i];
	}
	return plane == "foreground" ? self.getForegroundStructuresContainer() : (plane == "terrain" ? self.getBackgroundContainer() : self.getBackgroundBehindContainer());
}
function libUpkeep() {
	for (c in libsMade) {
		// (a picture shown on its own: its paints' container, where its code puts it)
		if (c.kind == "bitmap") {
			var d = c.bitmapData;
			if (d == null) continue;
			var root = pxRoot(d);
			if (c.into == null) { c.into = libInto(c.plane); c.into.addChild(root); }
			var bgs = stage == null || stage.exports == null ? null : stage.exports.__cameraBgs;
			var cam = bgs != null && bgs.indexOf(c.plane) >= 0;
			root.x = (cam ? 0.0 : (c.part == null ? 0.0 : c.part.originX())) + c.x * SIZE;
			root.y = (cam ? 0.0 : (c.part == null ? 0.0 : c.part.originY())) + c.y * SIZE;
			root.scaleX = c.scaleX * SIZE;
			root.scaleY = c.scaleY * SIZE;
			root.visible = c.visible != false;
			root.alpha = c.alpha;
			continue;
		}
		if (c.vs.length == 0) {
			var into = libInto(c.plane);
			for (p in c.parts) {
				var v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(p.id), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
				if (v == null) continue;
				into.addChild(v.getViewRootContainer());
				c.vs.push({ v: v, ax: p.ax, ay: p.ay, w: p.w });
			}
		}
		// where it is: its place in the plane it was put in, from where the tree placed that plane
		var ox = c.part == null ? 0.0 : c.part.originX();
		var oy = c.part == null ? 0.0 : c.part.originY();
		for (e in c.vs) {
			e.v.setX(ox + c.x * SIZE + (c.scaleX < 0 ? -(e.ax + e.w) : e.ax));
			e.v.setY(oy + c.y * SIZE + e.ay);
			e.v.setScaleX(c.scaleX);
			e.v.setScaleY(c.scaleY);
			e.v.setVisible(c.visible != false);
			e.v.setAlpha(c.alpha);
		}
		// (the frame it shows, the source's: two of this engine's a frame)
		if (c.vs.length > 0) c.currentFrame = Std.int((c.vs[0].v.getCurrentFrame() - 1) / 2) + 1;
	}
}

// ---- camPoint
// a point the camera keeps in its shot for a while (`addTimedTargetPoint`, the source's units and
// frames), and the same point let go: the engine's own point for each the source's code gives it
var camPoints = [];
function camPoint(c, p, n) {
	if (c == null || p == null) return;
	var now = match.getElapsedFrames();
	var live = [];
	for (r in camPoints) if (r.until > now) live.push(r);
	camPoints = live;
	var q = Point.create(p.x * SIZE, p.y * SIZE);
	camPoints.push({ p: p, q: q, until: now + n * 2 });
	c.addTimedTargetPoint(q, n * 2);
}
function camPointGone(c, p) {
	for (r in camPoints) if (r.p == p) { c.deleteTimedTargetPoint(r.q); camPoints.remove(r); break; }
}

// ---- deathSwitch
// the death bounds' switches its code flips (`enableFallDeath`): the blast zone on that side (the
// bottom for falls, the top for the ceiling) KOs no one while off, and they are kept to read back
var deathOff = self.makeObject(null);
function deathSwitch(k, on) {
	if (deathOff.get() == null) deathOff.set(new haxe.ds.StringMap());
	deathOff.get().set(k, on != true);
	var t = on == true ? BlastZoneType.ALWAYS_KO : BlastZoneType.DISABLED;
	if (stage != null) stage.updateStageStats(k == "fall" ? { bottomBlastZoneType: t } : { topBlastZoneType: t });
}
function deathOn(k) { return deathOff.get() == null || deathOff.get().get(k) != true; }

// ---- foeOwnStats
// another fighter's own stats, as far as this engine can say them: its name (the one its damage
// counter shows); the picture it is shown by is its own package's, drawn nowhere here
function foeOwnStats(c) {
	var n = c == null || c.getDefaultDamageCounterName == null ? null : c.getDefaultDamageCounterName();
	return { statsName: n, displayName: n, thumbnail: null };
}

// ---- camPart
// a clip of the planes the camera keeps in its shot for a while (the source's frames), at where the
// part stands, and the same clip let go
var camParts = [];
function camPart(c, p, n) {
	if (c == null || p == null || p.at == null) return;
	camPartGone(c, p);
	var q = Point.create(p.at.x, p.at.y);
	camParts.push({ p: p, q: q });
	c.addTimedTargetPoint(q, n * 2);
}
function camPartGone(c, p) {
	for (r in camParts) if (r.p == p) { c.deleteTimedTargetPoint(r.q); camParts.remove(r); break; }
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

// ---- clipRun
// a clip of its library whose frames have code, made by its code: a clip record whose timeline runs
// a frame each of the source's frames (every second engine frame) while it plays, from its first,
// each frame's script run as the timeline enters it (`frames`, the class's own, handed the record).
// a jump by label or number goes there and runs that frame's script; a label the clip lacks goes
// nowhere, as the source's player stops there with an error
var clipsRunning = [];
var clipsTicking = false;
function clipRun(r, total, labels, frames) {
	r.totalFrames = total;
	r.labelList = labels;
	r.frames = frames;
	r.currentFrame = 1;
	r.currentLabel = clipLabelAt(r, 1);
	r.playing = true;
	r.entered = false;
	clipsRunning.push(r);
	if (!clipsTicking) { clipsTicking = true; self.addTimer(2, -1, clipsStep, { persistent: true }); }
	return r;
}
function clipLabelAt(r, n) {
	var l = null;
	for (p in r.labelList) if (p[0] <= n) l = p[1];
	return l;
}
function clipEnter(r) {
	r.entered = true;
	r.currentLabel = clipLabelAt(r, r.currentFrame);
	if (r.frames != null) r.frames(r, r.currentFrame);
}
function clipsStep() {
	for (r in clipsRunning.copy()) {
		if (!r.entered) { clipEnter(r); continue; }
		if (!r.playing) continue;
		r.currentFrame = r.currentFrame >= r.totalFrames ? 1 : r.currentFrame + 1;
		clipEnter(r);
	}
}
function clipGo(r, to, stop) {
	if (r == null || r.totalFrames == null) return;
	var n = null;
	if (Std.isOfType(to, String)) { for (p in r.labelList) if (p[1] == to) { n = p[0]; break; } }
	else n = Std.int(to);
	if (n == null) return;
	n = n < 1 ? 1 : (n > r.totalFrames ? r.totalFrames : n);
	r.playing = !stop;
	// (to the frame it is on: nothing is entered, so no script runs again)
	if (n == r.currentFrame && r.entered) return;
	r.currentFrame = n;
	clipEnter(r);
}
function clipStep(r, d) {
	if (r == null || r.totalFrames == null) return;
	clipGo(r, r.currentFrame + d, true);
}
function clipPlay(r, on) { if (r != null) r.playing = on; }

// ---- clipAdd
// a clip put in another (its code's `addChild`), and taken out
function clipAdd(c, k) {
	if (c != null && k != null && c.kids != null && c.kids.indexOf(k) < 0) c.kids.push(k);
	// (the one it is in, asked for when its place on the screen is: a function, so no record holds
	// the one it is in, nor that one it)
	if (k != null && c != null && (k.kids != null || k.lib == true)) k.parentOf = function() { return c; };
	// (put on the screen, or in a clip on it: drawn there, `screenClip`)
	if (k != null && c != null && (c.screen == true || c.onScreen == true)) screenShow(k);
	return k;
}
// a point in a clip record's own space, in the screen's: its place and every one it is in, added
function clipGlobal(c, p) {
	var x = p == null ? 0.0 : p.x;
	var y = p == null ? 0.0 : p.y;
	var at = c;
	var guard = 0;
	while (at != null && guard < 32) {
		x += at.x == null ? 0 : at.x;
		y += at.y == null ? 0 : at.y;
		at = at.parentOf == null ? null : at.parentOf();
		guard++;
	}
	return { x: x, y: y };
}
// a clip it holds by name, as its code looks one up
function clipChild(c, n) {
	if (c == null || c.kids == null) return null;
	for (k in c.kids) if (k != null && k.name == n) return k;
	return null;
}
function clipRemove(c, k) {
	if (c != null && c.kids != null) c.kids.remove(k);
	if (k != null && k.onScreen == true) screenHide(k);
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

// ---- hurtInterrupt
// the check an object's code makes of each hit about to land on it (`setHurtInterrupt`): handed
// the one hitting it, as `target`, and a hit it answers true is not landed; null takes the check
// away. the hit is turned away where it is dealt, so every object in the match that can hit is
// listened to as it comes in. the checks are kept in the match's globals (another object's own
// exports may be the engine's, with no room for them)
function hurtChecks() {
	if (match.globals.__hurtChecks == null) { match.globals.__hurtChecks = []; match.globals.__hurtWatched = []; }
	return match.globals.__hurtChecks;
}
function hurtInterrupt(o, f) {
	if (o == null) return;
	var all = hurtChecks();
	var mine = null;
	for (h in all) if (h.o == o) mine = h;
	if (mine == null) { mine = { o: o, f: f }; all.push(mine); } else mine.f = f;
	var watch = function() {
		var seen = match.globals.__hurtWatched;
		var objs = [];
		for (c in match.getCharacters()) objs.push(c);
		for (p in match.getProjectiles()) objs.push(p);
		for (g in match.getCustomGameObjects()) objs.push(g);
		for (a in objs) {
			if (seen.indexOf(a) >= 0) continue;
			seen.push(a);
			var by = a;
			by.addEventListener(GameObjectEvent.HITBOX_CONNECTED, function(e) {
				var foe = e.data.foe;
				if (foe == null) return;
				for (h in match.globals.__hurtChecks) if (h.o == foe && h.f != null && h.f({ target: by, attackBoxData: e.data.hitboxStats, collisionRect: null }) == true) { e.data.hitboxStats.disabled = true; return; }
			}, { persistent: true });
		}
	};
	watch();
	o.addTimer(1, -1, watch, { persistent: true });
}

// ---- stageFocus
// the camera frames the whole stage for `n` of the source's frames, its fighters not followed
// (`CameraMode.STAGE`), then follows them again; a call while it does starts the count over, and
// 0 ends it now. the count is kept on the stage, which outlives whoever asked
function stageFocus(n) {
	var c = match.getCamera();
	if (c == null || stage == null) return null;
	var left = n == null ? 0 : Std.int(n * 2);
	if (left > 0 && c.getMode() != CameraMode.STAGE) c.setMode(CameraMode.STAGE);
	stage.exports.__focusLeft = left;
	if (left <= 0) { if (c.getMode() == CameraMode.STAGE) c.setMode(CameraMode.NORMAL); return null; }
	if (stage.exports.__focusTimed == true) return null;
	stage.exports.__focusTimed = true;
	stage.addTimer(1, -1, function() {
		var k = stage.exports.__focusLeft;
		if (k == null || k <= 0) return;
		k = k - 1;
		stage.exports.__focusLeft = k;
		if (k <= 0 && match.getCamera().getMode() == CameraMode.STAGE) match.getCamera().setMode(CameraMode.NORMAL);
	}, { persistent: true });
	return null;
}

// ---- boxesMeet
// where one object's boxes of a kind meet another's (`hitboxTest`), the kinds by the source's
// numbering: its attack box is this engine's hit box, its hit box this engine's hurt box, and its
// custom box the custom box b it is written as; a kind with no box here meets nothing
function boxesMeet(a, ka, b, kb) {
	if (a == null || b == null || a.isDisposed() || b.isDisposed()) return [];
	var ta = boxKind(ka);
	var tb = boxKind(kb);
	if (ta == null || tb == null) return [];
	var r = a.hitTestEntity(b, ta, tb);
	return r == null ? [] : r;
}

function boxKind(k) {
	switch (k) {
		case 0: return CollisionBoxType.HIT;
		case 1: return CollisionBoxType.HURT;
		case 2: return CollisionBoxType.GRAB;
		case 3: return CollisionBoxType.GRABHOLD;
		case 6: return CollisionBoxType.ABSORB;
		case 7: return CollisionBoxType.COUNTER;
		case 10: return CollisionBoxType.REFLECT;
		case 12: return CollisionBoxType.LEDGEGRAB;
		case 22: return CollisionBoxType.CUSTOMB;
	}
	return null;
}

// ---- pickUpNear
// an item lying free within a fighter's reach taken up by its own code (`pickupItem`), as a
// press of attack would take it: whether one was
function pickUpNear(c) {
	if (c == null) return false;
	for (o in match.getCustomGameObjects()) {
		if (o.isDisposed() || o.exports == null || o.exports.__takeBy == null) continue;
		if (o.exports.__takeBy(c) == true) return true;
	}
	return false;
}

// ---- unseenFor
// a fighter not drawn for `n` of the source's frames (`setInvisibilityTimer`), then drawn again
function unseenFor(c, n) {
	if (c == null || c.isDisposed()) return null;
	c.setVisible(false);
	var k = n == null ? 0 : Std.int(n * 2);
	if (k <= 0) { c.setVisible(true); return null; }
	c.addTimer(k, 1, function() { if (!c.isDisposed()) c.setVisible(true); }, { persistent: true });
	return null;
}

// ---- paletteOn
// an object or a clip drawn in other colours by its code (`setPaletteSwapData`, `replacePalette`):
// each colour the swap lists (`colors`) drawn as its replacement, by the engine's palette shader;
// the swap it was given before is taken off, and a clip this engine does not draw is left be
var paletteShaders = self.makeObject(null);
function paletteOn(o, pal) {
	// (a part's record: the effect it is drawn as, `projPart`)
	if (o != null && o.addShader == null && o.vfx != null) o = o.vfx;
	if (o == null || o.addShader == null || o.isDisposed()) return;
	if (paletteShaders.get() == null) paletteShaders.set(new haxe.ds.StringMap());
	var key = Std.string(o.getUid());
	var was = paletteShaders.get().get(key);
	if (was != null) o.removeShader(was);
	var from = pal == null ? null : pal.colors;
	var to = pal == null ? null : pal.replacements;
	if (from == null || to == null) { paletteShaders.get().remove(key); return; }
	var m = [0 => 0];
	m.remove(0);
	var any = false;
	for (i in 0...from.length) {
		if (i >= to.length || from[i] == to[i]) continue;
		m.set(argb(from[i]), argb(to[i]));
		any = true;
	}
	if (!any) { paletteShaders.get().remove(key); return; }
	var sh = PaletteSwapShader.create(m);
	o.addShader(sh);
	paletteShaders.get().set(key, sh);
}
// (a costume's record, as `setPaletteSwapData` is handed one: its swap is its `paletteSwap`)
function paletteDataOn(o, d) {
	paletteOn(o, d == null ? null : d.paletteSwap);
}
// a colour as the shader keys it, opaque where the source gives no alpha
// (worked in floats: a colour above the int range would not survive Std.int)
function argb(c) {
	var f = c * 1.0;
	if (f >= 0 && f < 16777216.0) f = f + 4278190080.0;
	if (f > 2147483647.0) f = f - 4294967296.0;
	return Std.int(f);
}

// ---- fsCutscene
// a final smash cutscene (`triggerFSCutscene`): its clip, an object of the fighter's, holds itself
// over the screen at its bottom centre (`__screenPinned`, its own update) until it ends itself or is
// ended. which one plays is kept on the fighter, so any of its objects can start it, ask after it or
// end it
function fsFighter() { var o = self.getRootOwner(); return o != null ? o : self; }
function fsCutsceneStart(e) {
	if (e == null) return;
	// (it plays until its own code ends it, not for an object's usual life)
	if (e.exports != null && e.exports.__keepAlive != null) e.exports.__keepAlive();
	if (e.exports != null) e.exports.__screenPinned = true;
	fsFighter().exports.__fsClip = e;
}
function fsCutscenePlaying() {
	var e = fsFighter().exports.__fsClip;
	return e != null && !e.isDisposed();
}
function fsCutsceneEnd() {
	var e = fsFighter().exports.__fsClip;
	if (e != null && !e.isDisposed()) e.kill();
	fsFighter().exports.__fsClip = null;
}

// ---- pinToScreen
// held over the screen at its bottom centre, a source pixel to a screen pixel whatever the
// camera's zoom, once its maker asks (`__screenPinned`: a final smash cutscene); where its own code
// moved it since it was last held there stays (its shake)
var pinX = self.makeObject(null);
var pinY = self.makeObject(null);
var offX = self.makeFloat(0);
var offY = self.makeFloat(0);
function pinToScreen() {
	if (self.exports.__screenPinned != true) return;
	var c = match.getCamera();
	var z = c.getZoomScaleX();
	if (z == null || z <= 0) z = 1;
	self.setXSpeed(0);
	self.setYSpeed(0);
	if (pinX.get() != null) { offX.set(offX.get() + (self.getX() - pinX.get())); offY.set(offY.get() + (self.getY() - pinY.get())); }
	pinX.set(c.getX() + offX.get() / z);
	pinY.set(c.getY() + c.getViewportHeight() / 2 / z + offY.get() / z);
	self.setX(pinX.get());
	self.setY(pinY.get());
	self.setScaleX(1 / (z * SIZE));
	self.setScaleY(1 / (z * SIZE));
}

// ---- selfDestructHeard
// a fighter knocked out by its own fall (`charSelfDestruct`): the engine's knock-out, told only when
// the fighter's count of self-destructs went up with it (the engine counts it before it tells)
function selfDestructHeard(c, h) {
	if (c == null || c.getScore == null) return;
	if (c.exports.__sdListeners == null) {
		c.exports.__sdListeners = [];
		c.exports.__sdSeen = c.getScore().selfDestructs;
		c.addEventListener(CharacterEvent.KNOCK_OUT, function(e) {
			var n = c.getScore().selfDestructs;
			var was = c.exports.__sdSeen;
			c.exports.__sdSeen = n;
			if (n > was) for (l in c.exports.__sdListeners.copy()) l(e);
		}, { persistent: true });
	}
	c.exports.__sdListeners.remove(h);
	c.exports.__sdListeners.push(h);
}
function selfDestructDrop(c, h) {
	if (c != null && c.exports != null && c.exports.__sdListeners != null) c.exports.__sdListeners.remove(h);
}

// ---- projectileById
// a projectile by its id (`getProjectile(id)`): the one in the match with that id, else none
function projectileById(id) {
	if (id == null) return null;
	for (p in match.getProjectiles()) if (!p.isDisposed() && p.getUid() == id) return p;
	return null;
}

// ---- matchStats
// a fighter's match so far, as the source's results record (`getMatchStatistics`): the score its
// damage counter shows, its knock-outs, falls and self-destructs, and the damage it gave and took
function matchStats(c) {
	if (c == null || c.getScore == null) return { score: 0, kos: 0, falls: 0, selfDestructs: 0, damageDealt: 0, damageTaken: 0 };
	var s = c.getScore();
	var given = 0.0;
	var taken = 0.0;
	if (s.damageGivenTotal != null) for (d in s.damageGivenTotal) if (d != null) given += d;
	if (s.damageTakenTotal != null) for (d in s.damageTakenTotal) if (d != null) taken += d;
	return { score: c.getDamageCounterScore(), kos: s.kos, falls: s.falls, selfDestructs: s.selfDestructs, damageDealt: given, damageTaken: taken };
}

// ---- spawnSpots
// the stage's respawn beacons (`getSpawnPositionMCs`), as records of their places in the source's
// units: copies, as the source hands out its list
function spawnSpots() {
	var all = stage == null || stage.exports == null ? null : stage.exports.__spawns;
	var out = [];
	if (all != null) for (p in all) out.push({ x: p.x, y: p.y });
	return out;
}

// ---- playerSettings
// a player's settings changed by the stage's code (`updatePlayerSettings`): its respawn place, in
// the source's units and stage space, kept on the fighter, which is put there as the engine
// respawns it
function playerSettings(c, r) {
	if (c == null || r == null || c.exports == null) return null;
	if (r.x_respawn != null) c.exports.__respawnX = r.x_respawn;
	if (r.y_respawn != null) c.exports.__respawnY = r.y_respawn;
	if (c.exports.__respawnHeard != true && (r.x_respawn != null || r.y_respawn != null)) {
		c.exports.__respawnHeard = true;
		c.addEventListener(CharacterEvent.RESPAWN, function(e) {
			if (c.exports.__respawnX != null) c.setX((c.exports.__respawnX - ORIGIN_X) * SIZE);
			if (c.exports.__respawnY != null) c.setY((c.exports.__respawnY - ORIGIN_Y) * SIZE);
		}, { persistent: true });
	}
	return null;
}

// ---- regionRect
// a region the stage's tree places under a name, as a rectangle in the source's units (`x`, `y`,
// `width`, `height`, as the source's Rectangle reads); nothing where the stage has none
function regionRect(n) {
	var r = stage == null || stage.exports == null || stage.exports.__named == null ? null : stage.exports.__named(n);
	if (r == null || r.rect == null) return { x: 0, y: 0, width: 0, height: 0 };
	return { x: r.rect.x, y: r.rect.y, width: r.rect.w, height: r.rect.h };
}

// ---- formSwitch
// a fighter turning into another fighter of its package (`replaceCharacter`): its other form, which
// its merged conversion publishes (`__formSwitch`); one with no such form stays as it is
function formSwitch(c, id, ?attack, ?label) {
	if (c == null || c.exports == null || c.exports.__formSwitch == null) return null;
	c.exports.__formSwitch(id, attack, label);
	return null;
}

// ---- finalForm
// a fighter taking its final form (`activateFinalForm`): the special form its declaration names
// (`__finalForm`, published by its merged conversion), for the time it states, then its own again;
// the transformation told to its listeners (`charTransform`)
function finalForm(c) {
	if (c == null || c.exports == null || c.exports.__finalForm == null || c.exports.__formSwitch == null) return null;
	var f = c.exports.__finalForm;
	c.exports.__formSwitch(f.id, null, null);
	if (c.exports.__transformHeard != null) for (l in c.exports.__transformHeard.copy()) l({ caller: c });
	if (f.frames > 0) c.addTimer(f.frames, 1, function() {
		if (c.isDisposed()) return;
		c.exports.__formSwitch(f.back, null, null);
		if (c.exports.__transformHeard != null) for (l in c.exports.__transformHeard.copy()) l({ caller: c });
	}, { persistent: true });
	return null;
}

// ---- metalOn
// a fighter turned metal or back (`setMetalStatus`): the game's metal look (SATURATION, BRIGHTNESS,
// CONTRAST, as its package declares it) laid over it as the engine's colour filter, taken off as
// it turns back; its listeners for the change told (`charMetalChange`)
function metalOn(c, on) {
	if (c == null || c.exports == null) return null;
	var was = c.exports.__isMetal == true;
	var now = on == true;
	if (was == now) return null;
	if (now) {
		var h = new HsbcColorFilter();
		h.saturation = SATURATION;
		h.brightness = BRIGHTNESS;
		h.contrast = CONTRAST;
		c.addFilter(h);
		c.exports.__metalFilter = h;
	} else if (c.exports.__metalFilter != null) {
		c.removeFilter(c.exports.__metalFilter);
		c.exports.__metalFilter = null;
	}
	c.exports.__isMetal = now;
	if (c.exports.__metalHeard != null) for (l in c.exports.__metalHeard.copy()) l({ caller: c, isMetal: now });
	return null;
}

// ---- teamSet
// a team its code sets (`setTeamID`): kept in the match's globals, and a hit between it and an
// object of that team turned away where it is dealt (only a team above nothing counts, as the
// source's rule has it); `teamOf` is the team it set, else the engine's
function teamOf(o) {
	if (o == null) return -1;
	if (match.globals.__teams != null) for (t in match.globals.__teams) if (t.o == o) return t.id;
	return o.getTeam != null ? o.getTeam() : -1;
}
function teamSet(o, id) {
	if (o == null) return null;
	if (match.globals.__teams == null) { match.globals.__teams = []; match.globals.__teamWatched = []; }
	var mine = null;
	for (t in match.globals.__teams) if (t.o == o) mine = t;
	if (mine != null) { mine.id = id; return null; }
	match.globals.__teams.push({ o: o, id: id });
	var watch = function() {
		var seen = match.globals.__teamWatched;
		var objs = [];
		for (c in match.getCharacters()) objs.push(c);
		for (p in match.getProjectiles()) objs.push(p);
		for (g in match.getCustomGameObjects()) objs.push(g);
		for (a in objs) {
			if (seen.indexOf(a) >= 0) continue;
			seen.push(a);
			var by = a;
			by.addEventListener(GameObjectEvent.HITBOX_CONNECTED, function(e) {
				var foe = e.data.foe;
				if (foe == null) return;
				var t = teamOf(by);
				if (t > 0 && teamOf(foe) == t) e.data.hitboxStats.disabled = true;
			}, { persistent: true });
		}
	};
	watch();
	o.addTimer(1, -1, watch, { persistent: true });
	return null;
}

// ---- damageStep
// the damage an event tells of: the hit's own, or how far an update of its damage moved it (this
// engine's update tells the new total), from what it was when its code began listening
var damageWas = self.makeFloat(0);
var damageNow = self.makeFloat(0);
function damagePrime() { damageWas.set(self.getDamage()); damageNow.set(self.getDamage()); }
function damageStep(e) {
	if (e == null || e.data == null) return 0;
	if (e.data.hitboxStats != null) return e.data.hitboxStats.damage;
	if (e.data.damage == null) return 0;
	if (e.data.damage != damageNow.get()) { damageWas.set(damageNow.get()); damageNow.set(e.data.damage); }
	return damageNow.get() - damageWas.get();
}

// ---- standings
// the match's winners and losers as it stands: the fighters with the most stock left win (the most
// score, where they are even on stock), the rest lose
function standings(win) {
	var cs = match.getCharacters();
	var best = null;
	for (c in cs) { var v = c.getLives() * 100000 + c.getDamageCounterScore(); if (best == null || v > best) best = v; }
	var out = [];
	for (c in cs) if ((c.getLives() * 100000 + c.getDamageCounterScore() == best) == win) out.push(c);
	return out;
}

// ---- otherTimer
// a timer of another object's, kept by the function it runs so its code can take it off by that
// function (`removeTimer`), where this engine takes one off by its id
function otherTimer(o, n, t, h) {
	if (o == null) return null;
	var id = o.addTimer(n, t, h, { persistent: true });
	if (match.globals.__timers == null) match.globals.__timers = [];
	match.globals.__timers.push({ o: o, h: h, id: id });
	return id;
}
function otherUntimer(o, h) {
	var ts = match.globals.__timers;
	if (ts == null || o == null) return;
	for (t in ts.copy()) if (t.o == o && t.h == h) { if (!o.isDisposed()) o.removeTimer(t.id); ts.remove(t); }
}

// ---- screenClip
//: needs clipRecord clipRun clipAdd
// the HUD's layers (`getHUDBackgroundMC`, `getHUDForegroundMC`): the screen, its middle the
// layers' origin as the source's HUD sits, and a clip of the library its code makes
// (`getMCByLinkageName`) and puts there: the effect its clip is drawn as, on the camera's screen
// layer, a source pixel to a screen pixel, turned as its code turns it. its timeline runs as a clip
// record's (`clipRun`), each label one of the effect's animations (`anims`: name, first frame,
// frames, in the timeline's order)
var screenShown = [];
var screenTicking = false;
function screenLayer() { var r = clipRecord(null); r.screen = true; return r; }
function screenClip(id, anims, ?frames) {
	var r = clipRecord(id);
	r.anims = anims;
	var total = 0;
	var labels = [];
	for (a in anims) { labels.push([a[1], a[0]]); if (a[1] + a[2] - 1 > total) total = a[1] + a[2] - 1; }
	clipRun(r, total < 1 ? 1 : total, labels, frames);
	return r;
}
function screenShow(k) {
	if (k == null || k.onScreen == true) return;
	k.onScreen = true;
	if (k.id != null && k.anims != null && k.drawn == null) {
		var made = self.getResource().getContent(k.id);
		k.drawn = made == null ? null : Sprite.create(made);
		if (k.drawn != null) match.getCamera().getForegroundContainer().addChild(k.drawn);
	}
	screenShown.push(k);
	if (k.kids != null) for (c in k.kids) screenShow(c);
	if (!screenTicking) { screenTicking = true; self.addTimer(1, -1, screenUpkeep, { persistent: true }); }
}
function screenHide(k) {
	if (k == null || k.onScreen != true) return;
	k.onScreen = false;
	if (k.drawn != null) { match.getCamera().getForegroundContainer().removeChild(k.drawn); k.drawn = null; }
	screenShown.remove(k);
	if (k.kids != null) for (c in k.kids) screenHide(c);
}
function screenUpkeep() {
	for (k in screenShown) {
		var sp = k.drawn;
		if (sp == null) continue;
		var g = clipGlobal(k, null);
		var sx = k.scaleX;
		var sy = k.scaleY;
		var on = k.visible != false;
		var at = k.parentOf == null ? null : k.parentOf();
		var guard = 0;
		while (at != null && guard < 32) { sx *= at.scaleX == null ? 1 : at.scaleX; sy *= at.scaleY == null ? 1 : at.scaleY; if (at.visible == false) on = false; at = at.parentOf == null ? null : at.parentOf(); guard++; }
		sp.x = 320 + g.x;
		sp.y = 180 + g.y;
		sp.scaleX = sx / SIZE;
		sp.scaleY = sy / SIZE;
		sp.alpha = k.alpha;
		sp.visible = on;
		// (the frame its timeline is on: the animation of the label it is in, that far in)
		var f = k.currentFrame == null ? 1 : k.currentFrame;
		for (a in k.anims) if (f >= a[1] && f < a[1] + a[2]) {
			if (sp.currentAnimation != a[0]) sp.currentAnimation = a[0];
			sp.currentFrame = Std.int((f - a[1]) * sp.totalFrames / a[2]) + 1;
		}
	}
}

// ---- hitBoxMeta
// what the box an event's hit was made with declares of the source's that this engine has no stat
// for: its attacker's animation's metadata, by the box's number (`boxes`); none reads as nothing
function hitBoxMeta(e) {
	var f = e == null || e.data == null ? null : e.data.foe;
	var hb = e == null || e.data == null ? null : e.data.hitboxStats;
	if (f == null || hb == null || f.isDisposed()) return { none: true };
	var m = f.getAnimationStatsMetadata();
	var b = m == null ? null : m.boxes;
	if (b == null || hb.index == null || hb.index < 0 || hb.index >= b.length || b[hb.index] == null) return { none: true };
	return b[hb.index];
}

// ---- projPart
//: needs clipRecord clipRun
// a named child of its clip its code turns (a part): its art drawn apart, a vfx of the part's own
// timeline riding the object where its clip places it, and a record its code holds and turns as
// the source turns the clip (its place, turn, size, look, and its timeline, `clipRun`'s), which
// the vfx follows each frame. made as the object starts, and found again by its name
var partsShown = [];
function projPart(n, x, y, r, sx, sy, art, total, labels) {
	for (p in partsShown) if (p.name == n) return p;
	var p = clipRecord(null);
	p.name = n;
	p.x = x;
	p.y = y;
	p.rotation = r;
	p.scaleX = sx;
	p.scaleY = sy;
	p.art = art;
	p.vfx = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent(CONTENT), animation: art, loop: true, timeout: -1 }), self);
	if (p.vfx != null) p.vfx.pause();
	clipRun(p, total < 1 ? 1 : total, labels, null);
	if (partsShown.length == 0) {
		self.addEventListener(EntityEvent.DISPOSE, partsGone, { persistent: true });
		// (another object's code reaches its parts by name, through its clip: `partOf`)
		self.exports.__part = function(n) { for (q in partsShown) if (q.name == n) return q; return null; };
	}
	partsShown.push(p);
	partsFollow();
	return p;
}
function partsGone(e) {
	for (p in partsShown) if (p.vfx != null && !p.vfx.isDisposed()) p.vfx.destroy();
	partsShown = [];
}
function partsFollow() {
	if (partsShown.length == 0) return;
	var facing = self.isFacingRight() ? 1 : -1;
	var turn = self.getRotation() * Math.PI / 180;
	for (p in partsShown) {
		var v = p.vfx;
		if (v == null || v.isDisposed()) continue;
		var dx = p.x * SIZE;
		var dy = p.y * SIZE;
		v.setX(self.getX() + facing * (dx * Math.cos(turn) - dy * Math.sin(turn)));
		v.setY(self.getY() + dx * Math.sin(turn) + dy * Math.cos(turn));
		if (facing < 0) v.faceLeft(); else v.faceRight();
		v.setRotation(self.getRotation() + facing * p.rotation);
		v.setScaleX(p.scaleX * self.getScaleX());
		v.setScaleY(p.scaleY * self.getScaleY());
		v.setAlpha(p.alpha);
		v.setVisible(p.visible != false && self.getVisible());
		// (the frame its timeline is on: two of the engine's to each of the source's)
		var f = (p.currentFrame == null ? 1 : p.currentFrame) * 2 - 1;
		if (v.getAnimation() != p.art) v.playAnimation(p.art);
		if (v.getCurrentFrame() != f) v.playFrame(f);
		v.pause();
	}
}

// ---- bodyCopy
//: needs clipRecord clipAdd
//: its cases are each of the source's animation names, as the engine animation it is
// a copy of a fighter's body its code makes from the fighter's own clip
// (`getMCByLinkageName(f.getCharacterStat("linkage_id"))`): that fighter's own art, in its own
// colours, as a record its code places and turns. it is drawn where the clips it was put in place
// it, off the object that made it (or, put in none, where its code says on the stage), sized as
// its code sizes it, at the frame its stance is stepped to; a label is the fighter's animation of
// that name
var bodiesShown = [];
function bodyCopy(f) {
	var r = clipRecord(null);
	r.body = f;
	r.gone = false;
	var made = f == null || f.isDisposed() ? null : f.getGameObjectStat("spriteContent");
	r.sprite = made == null ? null : Sprite.create(made);
	if (r.sprite != null) {
		var shader = f.getCostumeShader();
		if (shader != null && shader.paletteMap != null) r.sprite.setPalette(shader.paletteMap);
		stage.getCharactersFrontContainer().addChild(r.sprite);
		r.sprite.visible = false;
	}
	r.stance = { currentFrame: 1, totalFrames: 1,
		playFrame: function(n) { r.stance.currentFrame = Std.int((n + 1) / 2); },
		pause: function() {}, resume: function() {} };
	r.isDisposed = function() { return r.gone; };
	r.destroy = function() { r.gone = true; if (r.sprite != null) stage.getCharactersFrontContainer().removeChild(r.sprite); bodiesShown.remove(r); };
	bodiesShown.push(r);
	if (bodiesShown.length == 1) self.addTimer(1, -1, bodiesFollow, { persistent: true });
	return r;
}
function bodyLabel(r, label, hold) {
	if (r == null || r.sprite == null) return;
	var anim = label;
	switch (label) {
		// {cases}
	}
	if (r.sprite.hasAnimation(anim)) r.sprite.currentAnimation = anim;
	r.stance.totalFrames = Std.int((r.sprite.totalFrames + 1) / 2);
	r.stance.currentFrame = 1;
}
function bodiesFollow() {
	for (r in bodiesShown) {
		var sp = r.sprite;
		if (sp == null || r.gone) continue;
		var g = clipGlobal(r, null);
		var facing = self.isFacingRight() ? 1 : -1;
		if (r.parentOf == null) { sp.x = g.x * SIZE; sp.y = g.y * SIZE; }
		else { sp.x = self.getX() + facing * g.x * SIZE; sp.y = self.getY() + g.y * SIZE; }
		sp.scaleX = (r.scaleX == null ? 1 : r.scaleX) * (r.body != null && !r.body.isDisposed() && !r.body.isFacingRight() ? -1 : 1);
		sp.scaleY = r.scaleY == null ? 1 : r.scaleY;
		sp.alpha = r.alpha;
		sp.visible = r.visible != false && !isDisposedSelf();
		sp.currentFrame = (r.stance.currentFrame - 1) * 2 + 1;
	}
}
function isDisposedSelf() { return self.isDisposed(); }

// ---- heardEarly
// the hits a projectile lands before its first frame's code has run (one made on a fighter hits it
// as it is made): kept from its start (`earlyListen`), and told to a listener for them its first
// frame adds, as the source's first frame listens before any hit
var earlyHits = [];
function earlyListen() {
	self.addEventListener(GameObjectEvent.HIT_DEALT, function(e) { if (!started.get()) earlyHits.push({ t: GameObjectEvent.HIT_DEALT, e: e, at: match.getElapsedFrames() }); }, { persistent: true });
	self.addEventListener(GameObjectEvent.SHIELD_HIT_DEALT, function(e) { if (!started.get()) earlyHits.push({ t: GameObjectEvent.SHIELD_HIT_DEALT, e: e, at: match.getElapsedFrames() }); }, { persistent: true });
}
// (told only as its first frame listens: a listener added later hears only the hits after it)
function heardEarly(t, h) { for (x in earlyHits.copy()) if (x.t == t && match.getElapsedFrames() - x.at <= 2) h(x.e); }

// ---- homingPick
// the target the source's engine picks for one that homes (`getHomingTarget`): the nearest fighter
// that is not itself, its owner or on its team, and not knocked out or dodging through the air,
// told to its `homingTarget` listeners as it changes
var homingHeard = [];
var homingLast = null;
function homingPick() {
	var me = self.getType() == EntityType.CHARACTER ? self : self.getRootOwner();
	var best = null;
	var bd = 0.0;
	for (c in match.getCharacters()) {
		if (c == null || c == me || c.isDisposed() || c.inStateGroup(CStateGroup.KO) || c.inStateGroup(CStateGroup.AIRDASH)) continue;
		if (me != null && c.getTeam() == me.getTeam()) continue;
		var d = Math.sqrt(Math.pow(c.getX() - self.getX(), 2) + Math.pow(c.getY() - self.getY(), 2));
		if (best == null || d < bd) { best = c; bd = d; }
	}
	if (best != homingLast) {
		homingLast = best;
		if (best != null) for (h in homingHeard.copy()) h({ data: { caller: self, target: best, type: "Character" } });
	}
	return best;
}

// ---- koPoint
// a knock-out a fighter is credited with (`charKOPoint`, on that fighter): told as another fighter is
// knocked out with its own count of knock-outs gone up
var koPointHeard = [];
function koPointListen(f, h) {
	if (f == null || f.getScore == null) return;
	for (k in koPointHeard.copy()) if (k.f == f && k.h == h) return;
	var rec = { f: f, h: h, count: f.getScore().kos, on: true };
	koPointHeard.push(rec);
	for (c in match.getCharacters()) if (c != f) c.addEventListener(CharacterEvent.KNOCK_OUT, function(e) {
		if (!rec.on || f.isDisposed()) return;
		var now = f.getScore().kos;
		if (now > rec.count) h({ data: { caller: f, victim: c } });
		rec.count = now;
	}, { persistent: true });
}
function koPointUnlisten(f, h) {
	for (k in koPointHeard.copy()) if (k.f == f && k.h == h) { k.on = false; koPointHeard.remove(k); }
}

// ---- attackCancel
// an attack of its cut off by another it starts (`charAttackCancelled`): told as one begins while
// another has not ended
var attackCancelHeard = [];
var attackCancelWatching = false;
var attackRunning = false;
function attackCancelListen(h) {
	attackCancelHeard.remove(h);
	attackCancelHeard.push(h);
	if (attackCancelWatching) return;
	attackCancelWatching = true;
	self.addEventListener(CharacterEvent.ATTACK_START, function(e) {
		if (attackRunning) for (k in attackCancelHeard.copy()) k({ data: { caller: self } });
		attackRunning = true;
	}, { persistent: true });
	self.addEventListener(CharacterEvent.ATTACK_END, function(e) { attackRunning = false; }, { persistent: true });
}

// ---- itemCreated
// a listener on the game for an item made (`gameItemCreated`): told by each item as it starts
// (`item.hx`), kept in the match's globals, which every object's code reaches
function itemCreatedListen(h, on) {
	if (match.globals.__itemCreatedHeard == null) match.globals.__itemCreatedHeard = [];
	var l = match.globals.__itemCreatedHeard;
	l.remove(h);
	if (on) l.push(h);
}

// ---- sizeStatus
// a fighter's size status (`setSizeStatus`): -1 half its size, 1 twice it, 0 its own, back to its
// own after ten seconds (three hundred source frames), its `charSizeChange` listeners told of each
// change; kept in the match's globals by the fighter, which every object's code reaches
function sizeRec(c) {
	if (match.globals.__sizes == null) match.globals.__sizes = [];
	for (r in match.globals.__sizes) if (r.c == c) return r;
	var r = { c: c, status: 0, base: c.getScaleX(), heard: [], timer: -1 };
	match.globals.__sizes.push(r);
	return r;
}
function sizeOf(c) { return c == null ? 0 : sizeRec(c).status; }
function sizeSet(c, s) {
	if (c == null || c.isDisposed()) return;
	var r = sizeRec(c);
	s = s < 0 ? -1 : (s > 0 ? 1 : 0);
	if (s == r.status) return;
	r.status = s;
	var k = s < 0 ? 0.5 : (s > 0 ? 2 : 1);
	c.setScaleX(r.base * k);
	c.setScaleY(r.base * k);
	if (r.timer >= 0) { c.removeTimer(r.timer); r.timer = -1; }
	if (s != 0) r.timer = c.addTimer(600, 1, function() { r.timer = -1; sizeSet(c, 0); }, { persistent: true });
	for (h in r.heard.copy()) h({ data: { caller: c, sizeStatus: s } });
}
function sizeListen(c, h, on) {
	if (c == null) return;
	var r = sizeRec(c);
	r.heard.remove(h);
	if (on) r.heard.push(h);
}

// ---- partOf
//: needs clipRun
// a named clip another object's clip places that its code turns, by that name: the part the object
// draws apart and publishes (`projPart`), or nothing where it has none of that name
function partOf(o, n) {
	if (o == null || o.isDisposed() || o.exports == null || o.exports.__part == null) return null;
	return o.exports.__part(n);
}
