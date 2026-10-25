// The runtime: the source's code running on this engine's object.
//
// The source runs at 30 frames a second and this engine at 60. its frames are doubled rather than
// clocked: every animation holds each source frame for two engine frames, its timers are the
// engine's at twice the frames, and the code it runs every frame (the object's update, its
// enter-frame listeners) runs whole on every second engine frame, as do the clips the engine does
// not play.
//
// Showing a frame: a frame set during the engine's update is advanced once before it is drawn,
// so setting frame F draws F+1. The engine's frames are the source's doubled (the source's frame
// k is the engine's 2k-1 and 2k, drawn the same), so the runtime sets 2k-1 on both engine frames
// and 2k is what is drawn: the source's frame, for both of its engine frames.
//
// When the engine changes the animation itself (a hit, a landing, a move it starts on an input),
// the runtime sees it at its next update and puts the source's clip where that animation is
// (`adopt`).

var PeptideRuntime = {};
PeptideRuntime.data = null;
PeptideRuntime.pieceIndex = null;
PeptideRuntime.firstFrames = null;
PeptideRuntime.projectileKnown = null;
PeptideRuntime.projectile = false;
PeptideRuntime.fm = null;
// the source's object this script runs, and its kind's upkeep (Fighters, Projectiles): what the
// source's engine does around an object of that kind
PeptideRuntime.object = null;
PeptideRuntime.kind = null;
// this script's life: on until its object ends. what the script leaves on other objects (a
// listener) holds it, and stops when it goes off
PeptideRuntime.life = { on: true };
PeptideRuntime.root = null;
PeptideRuntime.frame = 0;
PeptideRuntime.shown = null;
PeptideRuntime.depth = 0;
PeptideRuntime.globals = new haxe.ds.StringMap();
PeptideRuntime.reported = new haxe.ds.StringMap();
// frame scripts entered while the object is being made: they run at its first update, as a new
// clip's first frame does in the source (a projectile's maker can send it to a label first)
PeptideRuntime.making = false;
PeptideRuntime.held = [];
PeptideRuntime.fresh = false;

// each class's static initializer, run once before any of the content's code (the source's
// runs as the class is made): a static it works out (fairy glade's piranha spawn points) is
// otherwise left at its declared nothing
PeptideRuntime.staticsDone = false;
PeptideRuntime.statics = function(data) {
	if (PeptideRuntime.staticsDone || data.cinits == null) return;
	PeptideRuntime.staticsDone = true;
	for (f in data.cinits) PeptideRuntime.run(f);
};

PeptideRuntime.start = function(fm, data, kind) {
	PeptideRuntime.ended = false;
	PeptideRuntime.fm = fm;
	PeptideRuntime.data = data;
	PeptideRuntime.pieceIndex = null;
	PeptideRuntime.firstFrames = null;
	var o = data.object;
	PeptideRuntime.kind = kind;
	PeptideRuntime.statics(data);
	// the object: the engine class of its kind, with the class its package declares on top (a
	// package that declares only its clip's class still gets the engine's object around it, as
	// the source's engine wraps it)
	var me = {};
	o.cls.build(me);
	if (!AS3.is(me, "PeptideGameObject")) {
		me = {};
		kind.api().build(me);
		o.cls.build(me);
	}
	me.__fm = fm;
	fm.exports.__source = me;
	fm.exports.__linkageId = o.linkage;
	// its declared stats, for the scripts that ask it
	fm.exports.__declared = o.stats;
	PeptideRuntime.object = me;
	Rollback.decide();
	PeptideRuntime.kind.begin(me);
	PeptideRuntime.making = true;
	o.cls.init(me, null);
	var mc = PeptideRuntime.makeClip(o.clip);
	mc.__object = me;
	me.__mc = mc;
	PeptideRuntime.root = mc;
	// what the kind puts down once its clip is there (the stage: the bodies its tree placed, whose
	// code asks the stage's clip for its planes)
	if (PeptideRuntime.kind.made != null) PeptideRuntime.kind.made(me);
	// a kind whose code starts once the match is live runs it then (the stage: `Stages.boot`)
	if (PeptideRuntime.kind.initLate != true) PeptideRuntime.run(me.initialize);
	PeptideRuntime.making = false;

	// the engine's first animation is followed at the first update, once the frame scripts the
	// object's clips entered while it was made have run (the source runs them as it builds them,
	// before anything moves the clips on)
	PeptideRuntime.shown = null;
	PeptideRuntime.madeOn = PeptideRuntime.fm.getAnimation();
};

// whether the engine is in the script's update (a timer made there comes due a frame sooner)
PeptideRuntime.updating = false;
// whether the code running is a timer's (a timer made there counts its whole frames)
PeptideRuntime.inTimer = false;
// what comes due as the frame's script is done (a one-frame timer it started, Timers.create)
PeptideRuntime.afterFrame = [];
PeptideRuntime.frameDone = function() {
	while (PeptideRuntime.afterFrame.length > 0) {
		var due = PeptideRuntime.afterFrame;
		PeptideRuntime.afterFrame = [];
		for (f in due) f();
	}
};
PeptideRuntime.update = function() {
	if (PeptideRuntime.ended == true) return;
	Rollback.frame();
	PeptideRuntime.frame++;
	PeptideRuntime.updating = true;
	PeptideRuntime.clockedUpdate();
	PeptideRuntime.updating = false;
	PeptideRuntime.afterUpdate();
};
// what waits for the update it was asked in to finish (an object that ends itself mid-update has
// the rest of that update still to run on its fields, as the source's does)
PeptideRuntime.later = [];
PeptideRuntime.afterUpdate = function() {
	while (PeptideRuntime.later.length > 0) {
		var due = PeptideRuntime.later;
		PeptideRuntime.later = [];
		for (f in due) f();
	}
};

// ---- the engine's clock: its animations play, and their keyframes run the source's frames ------
//
// every object's entity carries the source's frames on its keyframes, and the engine plays it. a source frame shows as two engine frames, and the first one's keyframe runs the
// frame (`kf`) as the engine enters it: before that frame's update and hits. the source's timers
// and update run on the second engine frame of the frame before, so its order holds: timers,
// update, then the frame. an animation's first keyframe is its entry. a jump by the source's code
// moves the engine to the frame that shows where it landed (`pin`).

PeptideRuntime.lastEngineFrame = -1;
PeptideRuntime.lastEngineAnim = null;
PeptideRuntime.dirty = false;
// the animation the runtime itself last started (the engine's own entries are the engine's)
PeptideRuntime.shownBy = null;
// a jump the engine is being moved for: the frame it lands on (its keyframe runs its script)
PeptideRuntime.landing = -1;
// the runtime is switching the engine's animation for a jump
PeptideRuntime.switching = false;

// the clip whose playhead the engine's frame is
PeptideRuntime.clocked = function(clip) {
	return PeptideRuntime.data != null && clip == PeptideRuntime.shownClip() && !PeptideRuntime.leadIn(clip);
};
// a clip on a frame ahead of every piece of it (an unlabelled lead-in its labels start after, whose
// frame scripts set the object up): it plays on its own until it reaches one, as the source runs
// those frames before the first section the engine shows
PeptideRuntime.leadIn = function(clip) {
	// (an object's, a projectile's: a fighter or a stage plays on in its own animations)
	if (clip == null || clip.__sym == null || PeptideRuntime.kind.playsOut) return false;
	// each symbol's first drawn frame, worked out once: asked several times a frame
	var firsts = PeptideRuntime.firstFrames;
	if (firsts == null) {
		firsts = new haxe.ds.IntMap();
		for (q in PeptideRuntime.data.pieces) if (!firsts.exists(q.sym) || q.from < firsts.get(q.sym)) firsts.set(q.sym, q.from);
		PeptideRuntime.firstFrames = firsts;
	}
	var first = firsts.get(clip.__sym);
	return first != null && clip.currentFrame < first;
};

// a keyframe whose frame's script is written on it: the clip, when the engine is entering that
// frame of it (it moved on to it, or a jump the engine was just moved for landed on it), else null
PeptideRuntime.at = function(sym, f) {
	if (PeptideRuntime.ended == true || PeptideRuntime.root == null || PeptideRuntime.switching) return null;
	if (PeptideRuntime.fm.getAnimation() != PeptideRuntime.shown) return null;
	// another object's code jumped the clip since the engine last showed it (pit's down special
	// sends its orbitars to `end`): the engine goes where the clip is, and the frame it was
	// about to enter is no longer the clip's (the jump ran the frame it landed on)
	if (PeptideRuntime.dirty) { PeptideRuntime.pin(); return null; }
	var st = PeptideRuntime.shownClip();
	if (st == null || st.__sym != sym) return null;
	if (st.currentFrame == f) {
		if (PeptideRuntime.landing != f) return null;
		PeptideRuntime.landing = -1;
		return st;
	}
	MovieClip.arrive(st, f);
	return st;
};

// a frame's script from its keyframe, on the clip `at` handed back (none: not this clip's frame):
// what it throws is reported, and a jump it makes is settled
// a frame the engine entered while the runtime holds the object still (a hit the engine freezes
// nothing for, Fighters.frozen): its script waits for the hold to end, as the source's clip
// does not move on while it is frozen
PeptideRuntime.heldBack = function(f) {
	var me = PeptideRuntime.object;
	if (me == null || me.__holdUntil == null || PeptideRuntime.frame >= me.__holdUntil) return false;
	if (me.__heldFrames == null) me.__heldFrames = [];
	me.__heldFrames.push(f);
	return true;
};
PeptideRuntime.runAt = function(clip, script) {
	if (clip == null) return;
	if (PeptideRuntime.heldBack(function() { PeptideRuntime.runAt(clip, script); })) return;
	PeptideRuntime.run(script, clip);
	PeptideRuntime.frameDone();
	PeptideRuntime.settle();
};

// a keyframe: the engine entered the first engine frame of source frame `f` of clip `sym`
// (an animation's first keyframe enters it only when the runtime started it)
PeptideRuntime.kf = function(sym, f) {
	if (PeptideRuntime.ended == true || PeptideRuntime.root == null || PeptideRuntime.switching) return;
	if (PeptideRuntime.heldBack(function() { PeptideRuntime.kf(sym, f); })) return;
	// an animation the engine started is entered at the next update, once the engine has entered its
	// state (a walk's own starting speed is set by then, and the entry frame's script writes over it
	// as the source's does). its kind hears of it now, before the engine first moves it (an attack's
	// start takes effect from its first frame, as in the source)
	if (PeptideRuntime.fm.getAnimation() != PeptideRuntime.shown) {
		// the source's code played it (a jump compiled to the engine's own, which runs the frame
		// it lands on at once, as the source's jump does): the clip goes there now
		if (PeptideRuntime.depth > 0 && !PeptideRuntime.making && PeptideRuntime.jumped(sym, f)) return;
		// a landing the engine started: the source's clip goes to its landing frame as the fighter
		// touches down, and that frame's script runs before the frame's hits (mario's down air
		// readies its landing box there; taken at the next update, the box it had hit once more
		// and the landing box hit again)
		if (PeptideRuntime.data.isLanding != null && PeptideRuntime.data.isLanding(PeptideRuntime.fm.getAnimation()) == true) {
			PeptideRuntime.follow();
			return;
		}
		if (f == 1) PeptideRuntime.kind.entering(PeptideRuntime.object, PeptideRuntime.fm.getAnimation());
		return;
	}
	// a jump from another object's code goes first, as in `at`
	if (PeptideRuntime.dirty) { PeptideRuntime.pin(); return; }
	var st = PeptideRuntime.shownClip();
	// a frame the clip is already on ran when the clip got there (a jump landing on it)
	if (st == null || st.__sym != sym || st.currentFrame == f) return;
	MovieClip.enter(st, f);
	PeptideRuntime.frameDone();
	PeptideRuntime.settle();
};

// the engine went to an animation the source's code played: the clip is put on the frame it shows
// (its label under the root, then the frame, whose script runs)
PeptideRuntime.jumped = function(sym, f) {
	var anim = PeptideRuntime.fm.getAnimation();
	var p = PeptideRuntime.pieceOf(anim);
	if (p == null || p.sym != sym) return false;
	PeptideRuntime.shown = anim;
	PeptideRuntime.shownBy = anim;
	PeptideRuntime.dirty = false;
	var root = PeptideRuntime.root;
	if (p.label != "" && root.currentLabel != p.label) {
		PeptideRuntime.switching = true;
		root.gotoAndStop(p.label);
		PeptideRuntime.switching = false;
	}
	var st = PeptideRuntime.shownClip();
	if (st != null && st.__sym == sym && st.currentFrame != f) MovieClip.enter(st, f);
	if (st != null) st.__enteredOn = PeptideRuntime.frame;
	return true;
};

// the source's code jumped a clip: when it is the one an animation shows (or the root, whose
// label picks it), the engine follows once the code is done
PeptideRuntime.moved = function(clip) {
	if (PeptideRuntime.data == null) return;
	if (clip == PeptideRuntime.root || clip == PeptideRuntime.shownClip()) PeptideRuntime.dirty = true;
};

// the source's code stopped or restarted the clip an animation shows: the engine holds or plays on
PeptideRuntime.hold = function(clip, on) {
	if (!PeptideRuntime.clocked(clip) || PeptideRuntime.kind.playsOut) return;
	if (on) PeptideRuntime.fm.pause(); else PeptideRuntime.fm.resume();
};

// what the source's code asked of the engine, once it is done
PeptideRuntime.settle = function() {
	if (PeptideRuntime.dirty) PeptideRuntime.pin();
};

// the engine to the frame that shows where the clip is
PeptideRuntime.pin = function() {
	var st = PeptideRuntime.shownClip();
	// an object with no clip (a zone its code sizes), or none of whose frames its entity draws
	// (its art is the stage's), has no frames to show
	if (st == null || st.__sym == null || PeptideRuntime.data.pieces.length == 0) { PeptideRuntime.dirty = false; return; }
	var p = PeptideRuntime.pieceAt(st.__sym, st.currentFrame);
	if (p == null) {
		PeptideRuntime.dirty = false;
		// a frame ahead of every section the clip's frames are drawn in is a lead-in the source
		// passes through (an unlabelled first frame before its labels): nothing of it is drawn
		var first = 1000000;
		for (q in PeptideRuntime.data.pieces) if (q.sym == st.__sym && q.from < first) first = q.from;
		if (st.currentFrame < first) return;
		// ...and a frame that places nothing draws nothing (a stance's "inactive" with its box gone)
		var tl = st.__tl;
		var fi = Std.int(st.currentFrame) - 1;
		if (tl != null && fi >= 0 && fi < tl.frames.length && tl.frames[fi].length == 0) return;
		// a fighter's engine plays on in its own animation; an object with no ways on of its own
		// (a projectile) has nothing to show
		if (!PeptideRuntime.kind.playsOut) PeptideRuntime.report("the source goes to frame " + st.currentFrame + " of clip " + st.__sym + " (under " + PeptideRuntime.root.currentLabel + "), which no animation here shows");
		return;
	}
	var fm = PeptideRuntime.fm;
	var n = 2 * (st.currentFrame - p.from + 1) - 1;
	// nothing shown yet (its maker's code jumped it as it was made): the engine shows that frame
	// at once, as the source's hit test sees the clip where it went in the frame it was made, and
	// the first update takes the jump up (PeptideRuntime.follow)
	if (PeptideRuntime.shown == null) {
		if (fm.getAnimation() == p.anim && n > 1 && n < fm.getTotalFrames()) fm.playFrame(n);
		return;
	}
	if (fm.getAnimation() == p.anim) {
		var cur = fm.getCurrentFrame();
		if (cur == n || cur == n + 1) { PeptideRuntime.dirty = false; return; }
		// the first frame of an animation the engine entered itself is the engine's: the engine
		// is moved once it is past it
		if (cur <= 1 && PeptideRuntime.shownBy != p.anim) return;
		PeptideRuntime.dirty = false;
		// back to the first frame of the animation it shows: this engine is never set back onto an
		// animation's first frame, so an object with no ways on of its own (a projectile) starts
		// the animation again. set nowhere, it played out into the next clip section (bandana
		// dee's spear loops `loop` until its count runs out, and ran on into `continueGrabbed`)
		if (n <= 1 && !PeptideRuntime.kind.playsOut) {
			PeptideRuntime.shownBy = p.anim;
			PeptideRuntime.switching = true;
			PeptideRuntime.showAnimation(p.anim);
			PeptideRuntime.switching = false;
			PeptideRuntime.switched();
			return;
		}
		PeptideRuntime.landing = st.currentFrame;
		PeptideRuntime.playFrame(n, true);
		PeptideRuntime.landing = -1;
		return;
	}
	PeptideRuntime.dirty = false;
	PeptideRuntime.shown = p.anim;
	PeptideRuntime.shownBy = p.anim;
	// the new animation's first keyframe is not an arrival: the clip is already where it landed
	PeptideRuntime.switching = true;
	PeptideRuntime.showAnimation(p.anim);
	PeptideRuntime.switching = false;
	PeptideRuntime.switched();
	if (n > 1) {
		PeptideRuntime.landing = st.currentFrame;
		PeptideRuntime.playFrame(n, true);
		PeptideRuntime.landing = -1;
	}
};

// the engine to another animation of the clip it shows: the source is still in the same state, so
// the listeners its code put on the object hold. this engine lets go of one not added to persist
// whenever its animation changes, so each one held is put back (naruto's rasengan listens for
// its hit on the frame it jumps to, which is in the next animation here; the hit was never told)
PeptideRuntime.showAnimation = function(anim) {
	var fm = PeptideRuntime.fm;
	var o = PeptideRuntime.object;
	var held = [];
	if (o != null && o.__handlers != null) for (x in o.__handlers) {
		var ev = PeptideRuntime.data.engineEvent(AS3.str(x.type));
		if (ev != null && fm.hasEventListener(ev, x.h)) held.push({ ev: ev, h: x.h });
	}
	fm.playAnimation(anim);
	for (l in held) if (!fm.hasEventListener(l.ev, l.h)) fm.addEventListener(l.ev, l.h, { persistent: false });
};

// the clip's playhead to the engine's frame, for the frames with no script (a frame with one
// arrived through its keyframe)
// what the object's code wrote to its boxes reaches the animation it goes on to before that can
// hit (samus's bomb writes its blast and goes on to it in the same frame)
PeptideRuntime.switched = function() {
	if (PeptideRuntime.kind.afterSwitch != null) PeptideRuntime.kind.afterSwitch(PeptideRuntime.object);
};
PeptideRuntime.sync = function() {
	// a projectile is drawn and struck at the scale its code gives its own clip (samus's bomb sets
	// 1.75 on itself as it starts, and its blast reaches as far as that scale says)
	var root = PeptideRuntime.root;
	var fm0 = PeptideRuntime.fm;
	// what the object is does not change: asked once, not every frame
	if (PeptideRuntime.projectileKnown != fm0) { PeptideRuntime.projectileKnown = fm0; PeptideRuntime.projectile = PeptideRuntime.isProjectile(fm0); }
	if (root != null && PeptideRuntime.projectile) {
		var sx = AS3.num(root.scaleX);
		var sy = AS3.num(root.scaleY);
		if (root.__scaleBase == null) root.__scaleBase = [fm0.getScaleX(), fm0.getScaleY(), 1.0, 1.0];
		var b = root.__scaleBase;
		if (sx == sx && sy == sy && (sx != b[2] || sy != b[3])) {
			b[2] = sx; b[3] = sy;
			fm0.setScaleX(b[0] * Math.abs(sx));
			fm0.setScaleY(b[1] * Math.abs(sy));
		}
	}
	var st = PeptideRuntime.shownClip();
	var p = PeptideRuntime.pieceOf(PeptideRuntime.fm.getAnimation());
	if (st == null || p == null || st.__sym != p.sym || PeptideRuntime.leadIn(st)) return;
	var f = p.from + Std.int((PeptideRuntime.fm.getCurrentFrame() - 1) / 2);
	if (f >= p.from + p.frames) f = p.from + p.frames - 1;
	if (f != st.currentFrame) MovieClip.arrive(st, f);
	// an object whose engine has no ways on from an animation (a projectile) goes on to the one that
	// shows its clip's next frame once this one is played out; so does a fighter's animation whose
	// clip plays on into one its engine has no way to (a charge special let go runs into its attack)
	if ((!PeptideRuntime.kind.playsOut || p.on != null) && st.__playing && PeptideRuntime.fm.getCurrentFrame() >= PeptideRuntime.fm.getTotalFrames()) {
		var next = st.currentFrame >= st.totalFrames ? 1 : st.currentFrame + 1;
		var q = PeptideRuntime.pieceAt(st.__sym, next);
		if (q != null && q.anim != p.anim && (!PeptideRuntime.kind.playsOut || q.anim == p.on)) { PeptideRuntime.shown = q.anim; PeptideRuntime.shownBy = q.anim; PeptideRuntime.showAnimation(q.anim); PeptideRuntime.switched(); }
	}
};

PeptideRuntime.clockedUpdate = function() {
	var fm = PeptideRuntime.fm;
	// an object that starts a source frame late does nothing until then
	if (PeptideRuntime.kind.waits(PeptideRuntime.object)) return;
	// a new object's first update: the frame scripts held while it was made, on the clips they
	// were entered on (each clip's latest, when it is still on that frame)
	if (PeptideRuntime.held != null) {
		var held = PeptideRuntime.held;
		PeptideRuntime.held = null;
		PeptideRuntime.fresh = true;
		// sent somewhere else by its maker before this (a projectile told to play a label as it is
		// fired): its clips go there first, and a held frame they have left does not run, as a new
		// clip's first frame does not in the source once it is sent on before it plays
		if (PeptideRuntime.fm.getAnimation() != PeptideRuntime.madeOn) PeptideRuntime.follow();
		for (h in held) if (h.clip.__held == h && h.clip.currentFrame == h.frame) MovieClip.runFrame(h.clip, h.frame);
	}
	// the engine's own change of animation first: a jump still waiting from the clip it has left
	// is that clip's, and moot
	PeptideRuntime.follow();
	PeptideRuntime.settle();
	// held still by a hit: its timers wait out these frames (Timers.create)
	if (PeptideRuntime.kind.frozen(PeptideRuntime.object)) {
		var o = PeptideRuntime.object;
		o.__frozenFrames = (o.__frozenFrames == null ? 0 : o.__frozenFrames) + 1;
		Effects.show();
		return;
	}
	// the engine started the animation it shows over again (an attack pressed again before the
	// first was done): back on its first frame from further on, with no jump of the code's on the
	// way. the engine runs no first keyframe for it, so its kind hears of the new start here
	if (fm.getCurrentFrame() == 1 && PeptideRuntime.lastEngineFrame > 1 && fm.getAnimation() == PeptideRuntime.lastEngineAnim
			&& !PeptideRuntime.dirty && PeptideRuntime.landing < 0)
		PeptideRuntime.kind.entering(PeptideRuntime.object, fm.getAnimation(), true);
	PeptideRuntime.sync();
	PeptideRuntime.lastEngineFrame = fm.getCurrentFrame();
	PeptideRuntime.lastEngineAnim = fm.getAnimation();
	PeptideRuntime.sourceFrame();
	PeptideRuntime.settle();
	PeptideRuntime.follow();
	PeptideRuntime.kind.engineFrame(PeptideRuntime.object);
	Effects.show();
};

// the engine is playing an animation the runtime did not ask for: the source's clip goes there
PeptideRuntime.follow = function() {
	var anim = PeptideRuntime.fm.getAnimation();
	if (anim == PeptideRuntime.shown) return;
	// a clip still in its lead-in plays that out first: its frames set the object up, and the engine's
	// animation is taken up once the clip reaches a section it shows
	if (PeptideRuntime.leadIn(PeptideRuntime.shownClip())) return;
	// a jump made before it showed anything is its maker's, as it was made (simon's jab picks its
	// whip's pose as it fires it): the clip stays where that put it, and the engine goes there
	var first = PeptideRuntime.shown == null;
	PeptideRuntime.shown = anim;
	PeptideRuntime.shownBy = null;
	if (!first) PeptideRuntime.dirty = false;
	PeptideRuntime.adopt(anim);
	};

// the script ends: its timers stop, and so does the rest of it
PeptideRuntime.teardown = function(?fm) {
	// the instance that is ending, by its own engine object, and only once the engine has let it
	// go. the engine can hand the same engine object to a new instance later, so the object stops
	// naming this one as its source: nothing is handed the ended instance again
	var ending = fm == null ? null : fm.exports.__source;
	if (ending == null || !fm.isDisposed()) return;
	// and its script does nothing more: an update the engine still calls (a frame script held for
	// a first update that never came) would run on what ended
	if (ending == PeptideRuntime.object) { PeptideRuntime.ended = true; PeptideRuntime.life.on = false; }
	fm.exports.__source = null;
	fm.exports.__facing = null;
	fm.exports.__thrownRight = null;
};

// the code the source runs every frame, run every engine frame at half its step. the first update
// only finishes making the object (its held frame scripts ran as the update began); after that the
// playheads move on
PeptideRuntime.sourceFrame = function() {
	if (PeptideAPI.camera != null) PeptideCamera.refresh(PeptideAPI.camera);
	var me = PeptideRuntime.object;
	// the kind's own rules count the engine's frames (a press shows for one of them): every one
	PeptideRuntime.kind.frame(me);
	// the source's code runs once a source frame, whole, on every other engine frame (the first
	// update is one of them)
	if (PeptideRuntime.frame % 2 == 0) return;
	if (me.update != null) PeptideRuntime.run(me.update);
	Events.dispatch(me, "gameTickEnd");
	Events.dispatch(PeptideAPI, "gameTickEnd");
	for (o in Events.everyFrame.copy()) Events.dispatch(o, "enterFrame");
	// the first one finishes making the object: nothing moves on yet
	if (PeptideRuntime.fresh == true) { PeptideRuntime.fresh = false; return; }
	PeptideRuntime.kind.beforeTick(me);
	MovieClip.tick(PeptideRuntime.root);
	Effects.frame();
};

// run the source's code; what it throws is reported once and does not stop the rest
PeptideRuntime.run = function(f, ?arg) {
	if (f == null) return null;
	if (PeptideRuntime.depth > 64) { PeptideRuntime.report("jumps nested past 64 deep"); return null; }
	// the object's x speed around its code: a kind that answers a speed its code set is told
	// (a fighter's walk acts on it as the source's does); the others are not asked
	var fm = PeptideRuntime.depth == 0 ? PeptideRuntime.fm : null;
	var live = fm != null && !fm.isDisposed();
	var vx = live && PeptideRuntime.kind != null && PeptideRuntime.kind.speedsSet != null ? fm.getXSpeed() : null;
	// code a projectile runs is the source's projectile pass, and so is what it has another
	// object do for it (a cloud firing a bolt through its fighter): its owner is told while it runs
	var owner = live && PeptideRuntime.kind != null && PeptideRuntime.kind.order > 0 && fm.getOwner != null ? fm.getOwner() : null;
	var pass = owner != null ? owner.exports : null;
	if (pass != null) pass.__inPass = (pass.__inPass == null ? 0 : pass.__inPass) + 1;
	PeptideRuntime.depth++;
	var r = null;
	try { r = arg == null ? f() : f(arg); } catch (e:Dynamic) { PeptideRuntime.report(Std.string(e)); }
	PeptideRuntime.depth--;
	if (pass != null) pass.__inPass = pass.__inPass - 1;
	if (vx != null && !fm.isDisposed()) PeptideRuntime.kind.speedsSet(PeptideRuntime.object, fm.getXSpeed() != vx);
	// a jump the code made takes the engine there once it is done, whatever ran it (a timer, an
	// event): left for the next update, the engine's own next keyframe would put the clip back
	if (PeptideRuntime.depth == 0 && !PeptideRuntime.making) PeptideRuntime.settle();
	// what waited for the code to finish, when no update will run it (an event's handler)
	if (PeptideRuntime.depth == 0 && PeptideRuntime.updating != true) PeptideRuntime.afterUpdate();
	return r;
};

// a call handed around as a value, made once per object and name: a listener added with it is
// removed with it
PeptideRuntime.bound = function(o, n, f) {
	if (o.__bound == null) o.__bound = new haxe.ds.StringMap();
	var b = o.__bound.get(n);
	if (b == null) { b = f; o.__bound.set(n, b); }
	return b;
};

// what went wrong, once each: kept on the object's exports (`__errors`, read by the harness) and
// logged as an error where the engine lets a script log
PeptideRuntime.errors = [];
PeptideRuntime.report = function(what) {
	if (PeptideRuntime.reported.exists(what)) return;
	PeptideRuntime.reported.set(what, true);
	PeptideRuntime.errors.push(what);
	if (PeptideRuntime.fm != null) PeptideRuntime.fm.exports.__errors = PeptideRuntime.errors;
	try { Engine.log("[ Error ] [peptide] " + what); } catch (e:Dynamic) {}
};

// whether an engine object is a projectile (the stage has no type to ask: asking it took every
// converted stage's script down on its first frame)
PeptideRuntime.isProjectile = function(fm) {
	if (fm == null) return false;
	try { return fm.getType() == EntityType.PROJECTILE; } catch (err:Dynamic) { return false; }
};
// whether an engine object is a fighter (the stage is no game object and has no type to ask)
PeptideRuntime.isFighter = function(fm) {
	if (fm == null) return false;
	try { return fm.getType() == EntityType.CHARACTER; } catch (err:Dynamic) { return false; }
};

// something the source does that this engine has no way to do: said once, and kept on the
// object's exports (`__gaps`), apart from the errors
PeptideRuntime.gaps = [];
// an effect the code names that is neither carried nor a global one: a clip the package has (a
// class of its own) and the conversion left behind is an error; one the package has no clip for
// draws nothing in the source either (ness's code attaches a pk flash effect its package never
// had), so it is a gap, not an error
PeptideRuntime.noEffect = function(name) {
	var what = "no effect " + name + " (not carried, no global one)";
	var classes = PeptideRuntime.data == null ? null : PeptideRuntime.data.classes;
	if (classes != null) for (c in classes) if (c != null && c.name == name) { PeptideRuntime.report(what); return; }
	PeptideRuntime.gap(what + ": its package has no such clip either");
};
PeptideRuntime.gap = function(what) {
	if (PeptideRuntime.reported.exists(what)) return;
	PeptideRuntime.reported.set(what, true);
	PeptideRuntime.gaps.push(what);
	if (PeptideRuntime.fm != null) PeptideRuntime.fm.exports.__gaps = PeptideRuntime.gaps;
	try { Engine.log("[peptide] gap: " + what); } catch (e:Dynamic) {}
};

// a method asked of an object that has none by that name (its class is in another package): a gap,
// and nothing comes back
PeptideRuntime.missing = function(what) {
	PeptideRuntime.gap(what + ": asked of an object that has no such method here");
	return null;
};

// what the source's own code prints (its print and trace): logged, and no error
PeptideRuntime.say = function(what) {
	try { Engine.log("[peptide] " + what); } catch (e:Dynamic) {}
};

// a clip for a symbol: an object of the class the symbol is bound to (or a plain clip), on the
// symbol's timeline
// made (its constructor runs with no parent yet, as a timeline's child's does), then put where it
// goes and named, and only then on its first frame, whose script can ask for its parent
PeptideRuntime.makeClip = function(sym, ?parent, ?name, ?bind) {
	// a bitmap the timeline places: a picture whose data is its library class
	var bmp = PeptideRuntime.data.bitmaps == null ? null : PeptideRuntime.data.bitmaps.get(sym);
	if (bmp != null) {
		var data = BitmapData.create(bmp.w, bmp.h, true, 0);
		data.__linkage = bmp.name;
		var b = Bitmap.create(data);
		if (parent != null) b.parent = parent;
		if (name != null) { b.name = name; b.__auto = false; }
		if (bind != null) bind(b);
		return b;
	}
	var cls = PeptideRuntime.data.classes.get(sym);
	var mc = cls != null ? cls.create() : MovieClip.create();
	MovieClip.attach(mc, sym, true);
	if (parent != null) mc.parent = parent;
	if (name != null) { mc.name = name; mc.__auto = false; }
	if (bind != null) bind(mc);
	MovieClip.enter(mc, 1);
	return mc;
};

// ---- which engine animation shows which source frame ----------------------------------------

PeptideRuntime.pieceOf = function(anim) {
	// asked several times a frame: the pieces are indexed by animation once (the first that
	// names one, as a scan finds it)
	var index = PeptideRuntime.pieceIndex;
	if (index == null) {
		index = new haxe.ds.StringMap();
		for (p in PeptideRuntime.data.pieces) if (!index.exists(p.anim)) index.set(p.anim, p);
		PeptideRuntime.pieceIndex = index;
	}
	return anim == null ? null : index.get(anim);
};

// the clip whose frames the engine's animations show: what the root places as `stance`, else
// the root itself
PeptideRuntime.shownClip = function() {
	var root = PeptideRuntime.root;
	if (root == null) return null;
	var st = PeptideRuntime.stanceOf(root);
	return st != null ? st : root;
};

// the animation that shows a frame of a clip: the one the engine is playing, when it shows that
// frame; else the clip's own (not one borrowing its frames); else any that shows it
PeptideRuntime.pieceAt = function(sym, frame) {
	var own = null;
	var any = null;
	for (p in PeptideRuntime.data.pieces) {
		if (p.sym != sym || frame < p.from || frame >= p.from + p.frames) continue;
		if (p.anim == PeptideRuntime.shown) return p;
		if (own == null && !p.borrowed) own = p;
		if (any == null) any = p;
	}
	return own != null ? own : any;
};

// the engine is playing an animation: the root goes to the label it is under, the clip it shows
// to its first frame
PeptideRuntime.adopt = function(anim) {
	var p = PeptideRuntime.pieceOf(anim);
	// an animation of the engine's own the source has none of (a crouch's way out, where the
	// source stands straight up): the engine plays it, noted rather than an error
	if (p == null) { PeptideRuntime.say("the engine plays " + anim + ", which no source frame shows"); return; }
	var root = PeptideRuntime.root;
	var st = PeptideRuntime.shownClip();
	// already on a frame the animation shows (a playhead that moved on into it): it stays there
	if (st != null && st.__sym == p.sym && st.currentFrame >= p.from && st.currentFrame < p.from + p.frames) return;
	// the engine's own variant of what the clip already shows (a turn, an idle's fidget): the
	// source has no such move, so its clip plays on where it is. an attack the engine starts is a
	// new attack, and starts; on an object with no variants of the engine's but the animations it
	// borrows (a projectile's spawn), any other section is one it was told to play, and it goes there
	var same = st != null && st.__sym == p.sym && (p.label == "" || root.currentLabel == p.label);
	if (same && (PeptideRuntime.kind.variants == true || p.borrowed) && AS3.at(PeptideRuntime.data.pkg.attacks, p.label) == null) return;
	var before = st;
	if (p.label != "" && root.currentLabel != p.label) root.gotoAndStop(p.label);
	st = PeptideRuntime.shownClip();
	// a clip the label jump just placed has entered its first frame and run its script, which
	// may have sent it on (a move that picks its section): it stays where that put it. a clip
	// already there goes to the animation's first frame
	var placed = st != before && st != null && st.__sym == p.sym && p.from == 1;
	if (st != null && !placed && st.currentFrame != p.from) st.gotoAndPlay(p.from);
	// the source's first frame of the new state is the one its entry put the clip on (or the one
	// that frame's script jumped to): the clip moves on from the next source frame
	if (st != null) st.__enteredOn = PeptideRuntime.frame;
};


// a frame of the animation the engine is on now, short of its last: a source frame shows as the
// first of its two engine frames, so the last is the engine's own to reach. set there, the engine
// ends the animation where it stands, and ending a state on the frame it began stops the engine
// dead (an engine-only variant such as a turn can be shorter than the clip it borrows)
PeptideRuntime.playFrame = function(n, ?ours) {
	var fm = PeptideRuntime.fm;
	if (fm.getAnimation() != PeptideRuntime.shown) return;
	// the first frame of an animation the engine entered itself is the engine's: set there, a
	// state that began this frame can be ended on it, which stops the engine dead (measured: a
	// fighter turning while its clip showed a later frame)
	if (ours != true && fm.getCurrentFrame() <= 1) return;
	var total = fm.getTotalFrames();
	if (n > total - 1) n = total - 1;
	// nor its first frame, ever: the engine starts every animation there itself, and set back
	// onto it a state that began a frame ago can end on it (the same stop)
	if (n <= 1) return;
	fm.playFrame(n);
};

// ---- the source's fields, where the engine keeps state for a rollback -------------------------
//
// a field of a class of the content's is a container of the engine's (`me.__c_hp.get()`), so a
// rollback puts it back with everything else. while the script starts each one is made where it
// is needed; after that an object takes one from the pool the script made as it started, and
// gives it back once the object is gone from what the source shows. which ones are taken is kept
// in containers too, so a rollback also takes back the ones taken since. past the pool a field is
// a plain value (`box`), which a rollback does not put back.

PeptideRuntime.pool = null;
// the containers the objects take their fields' values into, made at the script's top level: the
// engine makes them only there, before it calls the script's initialize
PeptideRuntime.makePool = function(cells) {
	if (cells == null) cells = { num: 0, obj: 0 };
	PeptideRuntime.pool = { num: [], obj: [], numNext: self.makeInt(0), objNext: self.makeInt(0), free: self.makeObject([]) };
	for (i in 0...cells.num) PeptideRuntime.pool.num.push(self.makeFloat(0));
	for (i in 0...cells.obj) PeptideRuntime.pool.obj.push(self.makeObject(null));
};
// the fields past the pool, which are no containers
PeptideRuntime.boxes = [];
PeptideRuntime.cell = function(v) { return PeptideRuntime.take(false, v); };
PeptideRuntime.cellNum = function(v) { return PeptideRuntime.take(true, v); };
PeptideRuntime.take = function(num, v) {
	var p = PeptideRuntime.pool;
	var c = null;
	if (p == null) c = num ? self.makeFloat(0) : self.makeObject(null);
	else {
		var free = p.free.get();
		var i = free.length - 1;
		while (i >= 0 && free[i].num != num) i--;
		if (i >= 0) {
			c = free[i].c;
			var rest = free.copy();
			rest.splice(i, 1);
			p.free.set(rest);
		} else {
			var list = num ? p.num : p.obj;
			var next = num ? p.numNext : p.objNext;
			var n = next.get();
			if (n < list.length) { c = list[n]; next.set(n + 1); }
		}
	}
	if (c == null) {
		PeptideRuntime.boxes.push(v);
		PeptideRuntime.gap("more fields at once than the script keeps containers for: past them, a rollback leaves a field as it is");
		var b = PeptideRuntime.box(v);
		PeptideRuntime.boxes[PeptideRuntime.boxes.length - 1] = b;
		return b;
	}
	c.set(num && v == null ? 0 : v);
	return c;
};
// a value a rollback does not put back, read and written the way a container is
PeptideRuntime.box = function(v) {
	var b = { v: v, get: null, set: null };
	b.get = function() { return b.v; };
	b.set = function(x) { b.v = x; return x; };
	return b;
};
// what an object does when it is gone: its containers go back, holding its values as plain ones
PeptideRuntime.own = function(me, f, held, back) {
	if (me.__looses == null) me.__looses = [];
	me.__looses.push(f);
	if (me.__cellHolds == null) me.__cellHolds = [];
	me.__cellHolds.push([held, back]);
};
PeptideRuntime.loose = function(c, num) {
	var b = PeptideRuntime.box(c.get());
	if (PeptideRuntime.pool != null && PeptideRuntime.boxes.indexOf(c) < 0) PeptideRuntime.pool.free.set(PeptideRuntime.pool.free.get().concat([{ c: c, num: num }]));
	return b;
};
PeptideRuntime.release = function(me) {
	if (me == null || me.__looses == null) return;
	var fs = me.__looses;
	me.__looses = null;
	for (f in fs) f();
	if (me.__kids != null) for (k in me.__kids) PeptideRuntime.release(k.clip);
};

// the fields the runtime reads off the source's clips, kept in containers where the content
// declares them: the clip a fighter's clip places as its animation, and the frame label the move says
// it is in
PeptideRuntime.stanceOf = function(mc) {
	if (mc == null) return null;
	return mc.__c_stance != null ? mc.__c_stance.get() : mc.stance;
};
PeptideRuntime.xframeOf = function(st) {
	if (st == null) return null;
	return st.__c_xframe != null ? st.__c_xframe.get() : st.xframe;
};
PeptideRuntime.setXframe = function(st, v) {
	if (st.__c_xframe != null) st.__c_xframe.set(v); else st.xframe = v;
};

// ---- the source's global variables (one store per match, as the source keeps them) -----------

PeptideRuntime.getGlobal = function(n) { return PeptideRuntime.globals.get(n); };
PeptideRuntime.setGlobal = function(n, v) { PeptideRuntime.globals.set(n, v); };
