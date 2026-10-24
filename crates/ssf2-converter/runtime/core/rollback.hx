// A rollback: an online match puts every state container the engine keeps back to how it was some
// frames ago and plays those frames again. content's fields are containers (`me.__c_hp`), so
// they go back with the engine's. the runtime's own state is kept here, one record a frame, by the
// frame the tick container says the match is on: when a rollback takes the tick back, the record
// of that frame is put back before the update runs.
//
// the record is taken as the update begins, when everything the last frame did (its timers, its
// hits, its keyframes) is done, and filed under that frame: the frame the engine saved its own
// state at. the fields a game object keeps are the ones the runtime's files write
// (`Rollback.saveObject`, generated from them at conversion); a clip keeps its playhead, its
// children and its place.

var Rollback = {};
Rollback.tick = null;
Rollback.last = 0;
Rollback.kept = [];
// more frames than the engine rolls back
Rollback.depth = 32;
// whether the match can roll back at all: an offline match, or one that only delays inputs, keeps
// no records (2 is an online match that rolls back, 3 a local one that rolls back the same way)
Rollback.on = false;
Rollback.decide = function() {
	var c = match.getMatchSettingsConfig();
	var n = c == null ? 0 : c.netcodeType;
	Rollback.on = n == 2 || n == 3;
};

// made at the script's top level: the engine makes containers only there
Rollback.start = function() {
	Rollback.tick = self.makeInt(0);
	Rollback.last = 0;
};

// what the harness reads back (tools/tests/rollback_parity.py): the fighter as each frame of the
// match left it, filed by that frame, so a frame played again overwrites what it first recorded
Rollback.trace = null;
Rollback.traceFrom = 0;
// a frame played again has to come out as it did the first time wherever the engine's own state
// does: a frame whose engine state matches its first playing but whose runtime state does not is
// something the runtime keeps that was not put back. (the engine's own state can differ on a
// replay: an input that arrived since, which the first playing only predicted)
Rollback.replayed = 0;
Rollback.mismatches = [];
Rollback.engineOf = [];
Rollback.note = function(t) {
	var fm = PeptideRuntime.fm;
	if (fm == null || fm.isDisposed()) return;
	var st = PeptideRuntime.shownClip();
	var engine = fm.getAnimation() + "|" + fm.getCurrentFrame() + "|" + Math.round(fm.getX() * 10) / 10 + "|" + Math.round(fm.getY() * 10) / 10;
	var ours = PeptideRuntime.shown + "|" + (PeptideRuntime.root == null ? null : PeptideRuntime.root.currentLabel) + "|" + (st == null ? null : st.currentFrame);
	var i = t - Rollback.traceFrom;
	var was = Rollback.trace[i];
	if (was != null) {
		Rollback.replayed++;
		if (Rollback.engineOf[i] == engine && was != engine + "|" + ours && Rollback.mismatches.length < 40) Rollback.mismatches.push(i + ": " + was + " -> " + engine + "|" + ours);
	}
	Rollback.trace[i] = engine + "|" + ours;
	Rollback.engineOf[i] = engine;
};
Rollback.traceRead = function(from, n) {
	var out = [];
	var i = from;
	while (i < from + n && i < Rollback.trace.length) { out.push(Rollback.trace[i]); i++; }
	return out.join(";");
};

// every update, first. the tick counts the match's frames in any match (the harness's trace is
// filed by it); the records are kept only where the match can roll back
Rollback.frame = function() {
	if (Rollback.tick == null) return;
	var t = Rollback.tick.get();
	if (Rollback.on) {
		if (t != Rollback.last) {
			var r = Rollback.kept[t % Rollback.depth];
			if (r != null && r.frame == t) Rollback.load(r); else PeptideRuntime.gap("a rollback went further back than the runtime keeps");
		} else {
			Rollback.kept[t % Rollback.depth] = Rollback.save(t);
		}
	}
	if (Rollback.trace != null && t >= Rollback.traceFrom) Rollback.note(t);
	Rollback.tick.set(t + 1);
	Rollback.last = t + 1;
};

// a value as it is now, apart from what changes it later: a list or a map copied, anything else as is
Rollback.copy = function(v) {
	if (v == null || Std.isOfType(v, String) || Std.isOfType(v, Float) || Std.isOfType(v, Bool)) return v;
	// a list and a map both copy themselves (a map's type is no value here to test against)
	if (v.copy != null && (v.push != null || v.keys != null)) return v.copy();
	return v;
};

Rollback.saveClip = function(c) {
	var kids = [];
	if (c.__kids != null) for (k in c.__kids) kids.push({ depth: k.depth, sym: k.sym, clip: k.clip });
	return [c, c.currentFrame, c.currentLabel, c.__playing, c.__half, c.__part, kids, c.__enteredOn, c.__held,
		c.x, c.y, c.visible, c.alpha, c.rotation, c.scaleX, c.scaleY, c.parent, c.name, c.__loops, c.__was, c.__anim,
		c.__looses == null ? null : c.__looses.copy(), c.__cellHolds == null ? null : Rollback.cellsOf(c),
		c.__listeners == null ? null : c.__listeners.copy(), c.__timers == null ? null : c.__timers.copy(), c.xframe, c.currentFrameLabel,
		c.__onceAt == null ? null : c.__onceAt.copy(), c.__tlAt];
};
// which containers a clip's fields are in (once it is gone its fields are plain values)
Rollback.cellsOf = function(c) {
	if (c.__cellHolds == null) return null;
	var out = [];
	for (h in c.__cellHolds) out.push(h[0]());
	return out;
};
Rollback.loadClip = function(s) {
	var c = s[0];
	c.currentFrame = s[1]; c.currentLabel = s[2]; c.__playing = s[3]; c.__half = s[4]; c.__part = s[5];
	var kids = [];
	for (k in s[6]) kids.push({ depth: k.depth, sym: k.sym, clip: k.clip });
	c.__kids = kids;
	c.__enteredOn = s[7]; c.__held = s[8];
	c.x = s[9]; c.y = s[10]; c.visible = s[11]; c.alpha = s[12]; c.rotation = s[13]; c.scaleX = s[14]; c.scaleY = s[15];
	c.parent = s[16]; c.name = s[17]; c.__loops = s[18]; c.__was = s[19]; c.__anim = s[20];
	c.__looses = s[21] == null ? null : s[21].copy();
	if (s[22] != null) { var i = 0; for (h in c.__cellHolds) { h[1](s[22][i]); i++; } }
	c.__listeners = s[23] == null ? null : s[23].copy();
	c.__timers = s[24] == null ? null : s[24].copy();
	// the frame label a move's clip says it is in, where the content keeps it as a plain field
	if (c.__c_xframe == null) c.xframe = s[25];
	c.currentFrameLabel = s[26];
	c.__onceAt = s[27] == null ? null : s[27].copy();
	// where its frame last put it (a place the code then changed is the code's)
	c.__tlAt = s[28];
};
// every clip there is: the object's tree and the effects
Rollback.clips = function(c, out) {
	if (c == null) return;
	out.push(Rollback.saveClip(c));
	if (c.__kids != null) for (k in c.__kids) if (k.clip != null) Rollback.clips(k.clip, out);
};

Rollback.save = function(t) {
	var clips = [];
	Rollback.clips(PeptideRuntime.root, clips);
	for (e in Effects.live) Rollback.clips(e, clips);
	var objects = [];
	if (PeptideRuntime.object != null) objects.push([PeptideRuntime.object, Rollback.saveObject(PeptideRuntime.object)]);
	for (v in Objects.made) objects.push([v, Rollback.saveObject(v)]);
	var fm = PeptideRuntime.fm;
	return {
		frame: t,
		clips: clips,
		objects: objects,
		exports: fm == null ? null : Rollback.saveExports(fm.exports),
		rt: [PeptideRuntime.frame, PeptideRuntime.shown, PeptideRuntime.shownBy, PeptideRuntime.dirty, PeptideRuntime.landing,
			Rollback.copy(PeptideRuntime.held), PeptideRuntime.fresh, PeptideRuntime.ended, PeptideRuntime.lastEngineFrame, PeptideRuntime.root,
			PeptideRuntime.onceAt.copy(), PeptideRuntime.lastEngineAnim],
		live: Effects.live.copy(),
		everyFrame: Events.everyFrame.copy(),
		made: Objects.made.copy(),
		globals: Rollback.copy(PeptideRuntime.globals),
	};
};

Rollback.load = function(r) {
	var rt = r.rt;
	PeptideRuntime.frame = rt[0]; PeptideRuntime.shown = rt[1]; PeptideRuntime.shownBy = rt[2]; PeptideRuntime.dirty = rt[3];
	PeptideRuntime.landing = rt[4]; PeptideRuntime.held = Rollback.copy(rt[5]); PeptideRuntime.fresh = rt[6]; PeptideRuntime.ended = rt[7];
	PeptideRuntime.lastEngineFrame = rt[8]; PeptideRuntime.root = rt[9];
	PeptideRuntime.onceAt = rt[10].copy();
	PeptideRuntime.lastEngineAnim = rt[11];
	for (c in r.clips) Rollback.loadClip(c);
	for (o in r.objects) Rollback.loadObject(o[0], o[1]);
	if (r.exports != null && PeptideRuntime.fm != null) Rollback.loadExports(PeptideRuntime.fm.exports, r.exports);
	Effects.live = r.live.copy();
	Events.everyFrame = r.everyFrame.copy();
	Objects.made = r.made.copy();
	PeptideRuntime.globals = Rollback.copy(r.globals);
};
