// The source's static API (PeptideAPI.*): the match as a fighter's code sees it.

// class PeptideAPI
var PeptideAPI = { name: "PeptideAPI" };
PeptideAPI.isReady = function() { return true; };
// a package starting and ending its use of the API: this runtime is always there for it
PeptideAPI.init = function(?src) { return true; };
PeptideAPI.deinit = function() {};
// listeners on the game itself (the match-wide events, the end of each frame)
PeptideAPI.__listeners = [];
PeptideAPI.addEventListener = function(type, fn, ?opts) { Events.add(PeptideAPI, type, fn, opts); };
PeptideAPI.removeEventListener = function(type, fn) { Events.remove(PeptideAPI, type, fn); };
PeptideAPI.hasEventListener = function(type, ?fn) { return Events.has(PeptideAPI, type, fn); };
// an effect the game places: where it is told, in the stage's units, else on the object whose
// code asks for it (which also holds the effect's art)
PeptideAPI.createVfx = function(id, ?opts) {
	var at = function(k) { return AS3.at(opts, k); };
	var o = { x: at("x"), y: at("y"), scaleX: at("scaleX"), scaleY: at("scaleY"), rotation: at("rotation"),
		behind: at("behind"), loop: at("loop"), parentLock: false, absolute: at("x") != null || at("y") != null };
	return Effects.attach(PeptideRuntime.object, id, o);
};
PeptideAPI.createVfxOverlay = function(id, ?opts) { return PeptideAPI.createVfx(id, opts); };
PeptideAPI.print = function(s) { PeptideRuntime.say("print: " + AS3.str(s)); };
PeptideAPI.isDebug = function() { return false; };
// the object a clip belongs to: the fighter whose clip it is in
PeptideAPI.getCharacter = function(x) {
	// a character asked for by its id
	if (Std.isOfType(x, Float)) return PeptideAPI.characterByUid(x);
	var c = x;
	var n = 0;
	while (c != null && n < 64) {
		if (c.__object != null) return PeptideAPI.characterOf(c.__object);
		if (c.__fm != null) return c;
		c = c.parent;
		n++;
	}
	return PeptideAPI.characterOf(PeptideRuntime.object);
};
// the object a clip belongs to (the projectile whose clip it is, asked from inside it)
PeptideAPI.objectOf = function(x) {
	var c = x;
	var n = 0;
	while (c != null && n < 64) {
		if (c.__object != null) return c.__object;
		if (c.__fm != null) return c;
		c = c.parent;
		n++;
	}
	return PeptideRuntime.object;
};
// the projectile (or item) a clip belongs to: none when the clip is some other object's (a charge
// ball drawn in its fighter's own animation asks, and goes on only for a projectile's)
PeptideAPI.getProjectile = function(x) { var o = PeptideAPI.objectOf(x); return AS3.is(o, "PeptideProjectile") ? o : null; };
// the enemy a clip belongs to (one a fighter makes rides a projectile here, running its class)
PeptideAPI.getEnemy = function(x) { var o = PeptideAPI.objectOf(x); return AS3.is(o, "PeptideEnemy") ? o : null; };
PeptideAPI.getItem = function(x) { var o = PeptideAPI.objectOf(x); return AS3.is(o, "PeptideItem") ? o : null; };
// the fighter an object is: itself, or the one it belongs to
PeptideAPI.characterOf = function(o) {
	if (o == null || o.__fm == null) return o;
	// the stage is no game object and has no type to ask: what it fires is its own
	var t = null;
	try { t = o.__fm.getType(); } catch (err:Dynamic) { return o; }
	if (t == EntityType.CHARACTER) return o;
	var owner = o.__fm.getOwner == null ? null : o.__fm.getOwner();
	return owner == null ? o : Objects.view(owner);
};
PeptideAPI.random = function() { return AS3.random(); };
PeptideAPI.randomUnit = function() { return AS3.random(); };
PeptideAPI.randomInteger = function(a, b) { return a + Std.int(AS3.random() * (b - a + 1)); };
PeptideAPI.randomBetween = function(a, b) { return PeptideAPI.randomInteger(a, b); };
PeptideAPI.getElapsedFrames = function() { return Std.int(PeptideRuntime.frame / 2); };
// the match's clock, as the source asks it for the frames it has run
PeptideAPI.getGameTimer = function() { return { getCurrentTime: function() { return Std.int(match.getElapsedFrames() / 2); } }; };
PeptideAPI.shakeCamera = function(n) { match.getCamera().shake(AS3.num(n)); };
PeptideAPI.camera = null;
PeptideAPI.getCamera = function() {
	if (PeptideAPI.camera == null) { var c = {}; PeptideCamera.build(c); c.__fm = match.getCamera(); PeptideAPI.camera = c; }
	return PeptideAPI.camera;
};
PeptideAPI.addTimedCameraTarget = function(o, n) { PeptideCamera.addTimedTarget(PeptideAPI.getCamera(), o, n); };
PeptideAPI.deleteTimedCameraTarget = function(o) { PeptideCamera.deleteTimedCameraTarget(PeptideAPI.getCamera(), o); };
PeptideAPI.removeTimedCameraTarget = PeptideAPI.deleteTimedCameraTarget;
// the HUD here is this engine's: what the source's does when a fighter is behind it is its own
PeptideAPI.addHUDDetection = function(?o) { PeptideRuntime.gap("addHUDDetection: the HUD seeing what is behind it"); };
PeptideAPI.removeHUDDetection = function(?o) { PeptideRuntime.gap("removeHUDDetection: the HUD seeing what is behind it"); };
PeptideAPI.getCamBounds = function() { return PeptideStage.getCameraBounds(Stage.get()); };
PeptideAPI.getDeathBounds = function() { return PeptideStage.getDeathBounds(Stage.get()); };
PeptideAPI.lightFlash = function(?on) {};
PeptideAPI.playSound = function(id, ?loop) { return Sounds.play(PeptideRuntime.object, id); };
PeptideAPI.stopSound = function(id) { Sounds.stop(id); };
PeptideAPI.getStage = function() { return Stage.get(); };
PeptideAPI.gameVar = function(n) { return PeptideRuntime.getGlobal(n); };
PeptideAPI.setGameVar = function(n, v) { PeptideRuntime.setGlobal(n, v); };

// ---- the stage's surfaces (source units) ------------------------------------------------------
PeptideAPI.getPlatformBetweenPoints = function(a, b, ?o) { return (a == null || b == null) ? null : Stage.between(a.x, a.y, b.x, b.y, o); };
PeptideAPI.hitTestGroundBetweenPoints = function(a, b, ?o) { return (a == null || b == null) ? null : Stage.between(a.x, a.y, b.x, b.y, o); };
PeptideAPI.hitTestGround = function(x, y, ?o) { return Stage.between(x, AS3.num(y) - 1, x, AS3.num(y) + 1, o); };

// ---- knockback, the source's formula, in its units --------------------------------------------
PeptideAPI.knockbackOf = function(growth, base, wkb, damage, victimDamage, weight, ?charged) {
	var g = AS3.num(growth);
	var b = AS3.num(base);
	var d = AS3.num(damage);
	var t = AS3.num(victimDamage) + d;
	var w = weight == null ? 100 : AS3.num(weight);
	var kb = ((((t * 0.1 + d * t * 0.05) * 1.4 * (200 / (w + 100))) + 18) * (g * 0.01)) + b;
	return AS3.truthy(charged) ? kb * 1.2 : kb;
};
// a knockback's launch speed, source pixels per source frame (a base-only 40 launches at 8.549)
PeptideAPI.knockbackVelocityOf = function(k) { return AS3.num(k) * 0.213725; };

// ---- identity and the match -------------------------------------------------------------------
PeptideAPI.uid = 0;
PeptideAPI.generateUID = function() { PeptideAPI.uid++; return PeptideAPI.uid; };
PeptideAPI.getUID = PeptideAPI.generateUID;
PeptideAPI.getCharacters = function() { return Objects.views(match.getCharacters()); };
PeptideAPI.characterByUid = function(n) {
	for (c in match.getCharacters()) if (c.getUid() == n) return Objects.view(c);
	return null;
};
PeptideAPI.getPlayer = function(x) {
	// a player by number (from 1), as the match's fighters stand in player order
	if (Std.isOfType(x, Int) || Std.isOfType(x, Float)) {
		var ps = match.getCharacters();
		var i = Std.int(x) - 1;
		return (ps == null || i < 0 || i >= ps.length) ? null : Objects.view(ps[i]);
	}
	if (x != null && x.stop != null) x.stop();
	return PeptideAPI.getCharacter(x);
};
// a versus match, with nothing switched on that the source can ask about
PeptideAPI.getGameMode = function() { return -1; };
PeptideAPI.hasFeature = function(n) { return false; };
PeptideAPI.getQualitySettings = function() { return { weather: true, stage_effects: "enabled", ambient_lighting: "enabled", global_effects: true }; };
PeptideAPI.getWinners = function() { return []; };
PeptideAPI.getLosers = function() { return []; };
// the track playing: the stage's own, from the match's start (its position in ms, by the match's
// frames: this engine's music is its own, so where the source's would be is worked out). a stage
// times its phases by it (final destination)
PeptideAPI.getCurrentMusicInfo = function() {
	var st = PeptideRuntime.data == null || PeptideRuntime.data.stage == null ? null : PeptideRuntime.data.stage.music;
	if (st == null) return null;
	var p = match.getElapsedFrames() * 1000 / 60;
	// read both as fields and by name
	var o = { linkage: st, position: p };
	AS3.putKey(o, "linkage", st);
	AS3.putKey(o, "position", p);
	return o;
};
PeptideAPI.playMusic = function(?id, ?n) { PeptideRuntime.gap("playMusic: changing the match's music"); };
PeptideAPI.stopMusic = function(?a0) { PeptideRuntime.gap("stopMusic: changing the match's music"); };
PeptideAPI.getCostumeData = function(?name, ?costume, ?team) { return null; };
// a clip made by its linkage name, for the source's code to hold (never drawn)
PeptideAPI.clipByLinkage = function(n) { var c = MovieClip.create(); c.__linkage = AS3.str(n); return c; };

// ---- spawning ---------------------------------------------------------------------------------
PeptideAPI.getItems = function() {
	var out = [];
	for (p in match.getProjectiles()) if (p.exports != null && p.exports.__isItem == true && !p.isDisposed()) out.push(Objects.view(p));
	return out;
};
PeptideAPI.makeEnemy = function(c) {
	var n = Std.isOfType(c, String) ? c : c.name;
	var o = Projectiles.fire(PeptideAPI.characterOf(PeptideRuntime.object), n, 0, 0);
	// an enemy the source makes, carried by a projectile here (`__np`: not a projectile in the
	// source, as an item is not either; the harness leaves both out of its count)
	if (o != null && o.__fm != null && o.__fm.exports != null) { o.__fm.exports.__isEnemy = true; o.__fm.exports.__np = true; }
	return o;
};
// an item made on the stage at a spot (source units), held by no one: one this package has an
// entity for; any other (the game's random pick among all its items) is a gap here
PeptideAPI.makePickup = function(name, ?x, ?y) {
	var n = AS3.str(name);
	if (Items.content(n) == null) { PeptideRuntime.gap("makePickup: " + n + " (an item this package has no entity for)"); return null; }
	var o = PeptideAPI.makeEnemy(n);
	if (o == null) return null;
	o.__fm.exports.__isItem = true;
	o.__fm.exports.__np = true;
	var s = PeptideRuntime.data.units.size;
	if (x != null) o.__fm.setX(AS3.num(x) * s);
	if (y != null) o.__fm.setY(AS3.num(y) * s);
	return o;
};
PeptideAPI.placePickup = function(c) {
	var o = PeptideAPI.makeEnemy(c);
	if (o != null) { o.__fm.exports.__isItem = true; o.__fm.exports.__np = true; }
	return o;
};
// the damage an attack's box does with its charge in: the engine states it on the box it hit with
PeptideAPI.calculateChargeDamage = function(box) { var d = box == null ? null : AS3.at(box, "damage"); return d == null ? 0 : AS3.num(d); };
// a fighter's match record: this engine keeps its lives and damage, not the source's score
PeptideAPI.matchStatistics = function(c) {
	PeptideRuntime.gap("getMatchStatistics: the match's score");
	return { score: 0, kos: 0, falls: 0, selfDestructs: 0, damageDealt: 0, damageTaken: 0 };
};
// the machine's time: none here, so the one fixed moment `Date` keeps (runtime/flash/misc.hx)
PeptideAPI.getTimestamp = function() { PeptideRuntime.gap("getTimestamp: the machine's clock (a fixed moment here)"); return Date.create(Date.now); };
PeptideAPI.hitboxTest = function(a, at, b, bt) { PeptideRuntime.gap("hitboxTest: two objects' boxes against each other"); return []; };
// the match's settings: a versus match on none of the source's stages, with none of its special
// modes on (read as a bit string: every bit off)
PeptideAPI.getGameSettings = function() {
	// one entry a player, as the match's fighters stand: no player's tag is handed to a stage here
	var players = [];
	var cs = match.getCharacters();
	if (cs != null) for (c in cs) players.push({ name: "", team: c.getTeam(), costume: c.getCostumeIndex(), human: true });
	return { levelData: { stage: "", specialModes: { toString: function(?radix) { return "0"; } } }, playerSettings: players };
};
