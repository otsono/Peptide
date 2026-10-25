// Sounds and effects the source's code starts.

var Sounds = {};
Sounds.play = function(me, id, ?voice) {
	if (id == null) return -1;
	var fm = me == null ? null : me.__fm;
	if (fm == null) return -1;
	var content = fm.getResource().getContent(AS3.str(id));
	if (content == null) { PeptideRuntime.report("no sound " + AS3.str(id)); return -1; }
	var clip = AudioClip.play(content);
	if (voice == true) me.__lastVoice = clip; else me.__lastSound = clip;
	return 0;
};
// the last sound (or voice) an object played, stopped
Sounds.stopLast = function(me, voice) {
	var c = voice ? me.__lastVoice : me.__lastSound;
	if (c != null && c.stop != null) c.stop();
	if (voice) me.__lastVoice = null; else me.__lastSound = null;
};
Sounds.stop = function(id) {};
// the attack's own numbered sound (attackSound1_id, attackVoice2_id, ...)
Sounds.attack = function(me, prefix, n) {
	var a = Attacks.current(me);
	if (a == null) return -1;
	return Sounds.play(me, AS3.at(a, prefix + AS3.str(n == null || n < 1 ? 1 : n) + "_id"), prefix == "attackVoice");
};

// An effect the source attaches is its clip: the source's code keeps it, moves it, hides it, sends
// it to a frame, and its own frame scripts run. The engine's effect follows that clip: its frame
// (by the effect's animations, DATA.effects), and what the code sets on it. It plays through once
// (or `loop` times) and is gone, unless its clip stops itself; taken off its parent, it is gone.
var Effects = {};
Effects.live = [];

Effects.attach = function(me, id, ?opts, ?used) {
	var fm = me == null ? null : me.__fm;
	if (fm == null || id == null) return null;
	var name = AS3.str(id);
	// a shared effect plays this engine's global one
	var global = PeptideRuntime.data.globalVfx == null ? null : PeptideRuntime.data.globalVfx.get(name);
	var pieces = global != null ? null : PeptideRuntime.data.effects.get(name);
	// one this object neither carries nor has a global one for (a shared effect another fighter's
	// file defines) is not spawned: the engine does not check the name it is handed, and an effect
	// made on a name it has no content for draws the object's own first animation (a copy of the
	// fighter on its revival platform). its code still gets a clip to move about, drawn nowhere
	if (global == null && pieces == null) { PeptideRuntime.noEffect(name); return MovieClip.create(); }
	var content = global != null ? "global::vfx.vfx" : fm.getResource().getContent(name);
	if (content == null) { PeptideRuntime.report("no effect " + name); return MovieClip.create(); }
	var s = PeptideRuntime.data.units.size;
	var o = opts == null ? {} : opts;
	var ox = o.x == null ? 0 : AS3.num(o.x);
	var oy = o.y == null ? 0 : AS3.num(o.y);
	// the stage's own effect has no owner here (an effect rides a game object, and the stage is none)
	var ofStage = me.__type == "SSF2Stage";
	var locked = !ofStage && AS3.truthy(o.parentLock);
	var stats = { spriteContent: content, animation: global != null ? global : (pieces != null && pieces.length > 0 ? pieces[0].anim : "active") };
	if (o.scaleX != null) stats.scaleX = AS3.num(o.scaleX);
	if (o.scaleY != null) stats.scaleY = AS3.num(o.scaleY);
	if (AS3.truthy(o.behind)) stats.layer = VfxLayer.CHARACTERS_BACK;
	if (locked) {
		// riding its owner: this engine mirrors the offset with the owner, so the source's own
		// flip of it is taken back off
		stats.x = fm.flipX(ox * s);
		stats.y = oy * s;
		stats.relativeWith = true;
		stats.resizeWith = true;
		stats.flipWith = true;
	}
	if (ofStage) {
		var org = PeptideRuntime.data.origin;
		stats.x = (ox - (org == null ? 0 : AS3.num(org.x))) * s;
		stats.y = (oy - (org == null ? 0 : AS3.num(org.y))) * s;
	}
	var vfx = ofStage ? match.createVfx(new VfxStats(stats)) : match.createVfx(new VfxStats(stats), fm);
	if (vfx == null) return null;
	// one the code makes and lets go (the compiler says nothing takes it), drawn from one piece of a
	// clip with no code of its own, played once: the engine's effect alone, which ends itself as its
	// animation does. no clip is made for it and nothing follows it
	if (used == false && pieces != null && pieces.length == 1 && PeptideRuntime.data.classes.get(pieces[0].sym) == null && o.loop == null) {
		if (!locked) {
			var px = ofStage || AS3.truthy(o.absolute) ? ox : fm.getX() / s + ox;
			var py = ofStage || AS3.truthy(o.absolute) ? oy : fm.getY() / s + oy;
			vfx.setX(px * s);
			vfx.setY(py * s);
		}
		if (o.rotation != null) vfx.setRotation(AS3.num(o.rotation));
		return null;
	}
	var clip = pieces != null && pieces.length > 0 ? PeptideRuntime.makeClip(pieces[0].sym, Stage.layer) : MovieClip.create();
	// a clip with no timeline of its own here (an effect drawn from another package): a playhead
	// as long as the animations that show it
	if (clip.__tl == null && pieces != null) {
		var n = 1;
		for (p in pieces) if (p.from + p.frames - 1 > n) n = p.from + p.frames - 1;
		var frames = [];
		for (i in 0...n) frames.push([]);
		clip.__tl = { frames: frames, labels: [], bind: null };
		clip.totalFrames = n;
	}
	clip.parent = Stage.layer;
	clip.__vfx = vfx;
	clip.__pieces = pieces;
	// a global effect ends itself when the engine's animation does
	clip.__global = global != null;
	clip.__locked = locked;
	clip.__loops = o.loop == null ? 1 : AS3.num(o.loop);
	clip.__born = PeptideRuntime.frame;
	// where it is, in the source's units: the stage's, off its owner unless given as the place
	if (ofStage || AS3.truthy(o.absolute)) { clip.x = ox; clip.y = oy; } else { clip.x = fm.getX() / s + ox; clip.y = fm.getY() / s + oy; }
	if (!locked) { vfx.setX(clip.x * s); vfx.setY(clip.y * s); }
	if (o.rotation != null) { clip.rotation = AS3.num(o.rotation); vfx.setRotation(clip.rotation); }
	if (o.scaleX != null) clip.scaleX = AS3.num(o.scaleX);
	if (o.scaleY != null) clip.scaleY = AS3.num(o.scaleY);
	Effects.seen(clip);
	Effects.live.push(clip);
	return clip;
};

// what the engine's effect was last given, to hand on only what the code changes
Effects.seen = function(c) {
	c.__was = { x: c.x, y: c.y, visible: c.visible, alpha: c.alpha, rotation: c.rotation, scaleX: c.scaleX, scaleY: c.scaleY };
};

Effects.remove = function(c) {
	Effects.live.remove(c);
	c.parent = null;
	PeptideRuntime.release(c);
	if (c.__vfx != null && !c.__vfx.isDisposed()) c.__vfx.dispose();
};

// every source frame: each clip's playhead moves on (not on the frame it was made); one that would
// go round again has played its last time
Effects.frame = function() {
	for (c in Effects.live.copy()) {
		if (c.__vfx.isDisposed()) { Effects.live.remove(c); continue; }
		if (c.__born == PeptideRuntime.frame || c.__global == true) continue;
		if (c.__playing && c.currentFrame >= c.totalFrames) {
			c.__loops--;
			if (c.__loops == 0) { Effects.remove(c); continue; }
		}
		MovieClip.tick(c);
	}
};

// every engine frame: the engine's effect shows the clip's frame and takes what the code set
Effects.show = function() {
	var s = PeptideRuntime.data.units.size;
	for (c in Effects.live) {
		var v = c.__vfx;
		if (v.isDisposed()) continue;
		if (c.__pieces != null) {
			for (p in c.__pieces) {
				if (c.currentFrame < p.from || c.currentFrame >= p.from + p.frames) continue;
				if (c.__anim != p.anim) { v.playAnimation(p.anim); c.__anim = p.anim; }
				var n = 2 * (c.currentFrame - p.from + 1) - 1;
				var total = v.getTotalFrames();
				v.playFrame(n > total - 1 ? total - 1 : n);
				break;
			}
		}
		var w = c.__was;
		if (!c.__locked && (c.x != w.x || c.y != w.y)) { v.setX(AS3.num(c.x) * s); v.setY(AS3.num(c.y) * s); }
		if (c.visible != w.visible) v.setVisible(AS3.truthy(c.visible));
		if (c.alpha != w.alpha) v.setAlpha(AS3.num(c.alpha));
		if (c.rotation != w.rotation) v.setRotation(AS3.num(c.rotation));
		if (c.scaleX != w.scaleX) v.setScaleX(AS3.num(c.scaleX));
		if (c.scaleY != w.scaleY) v.setScaleY(AS3.num(c.scaleY));
		Effects.seen(c);
	}
};

Effects.animationOf = function(id) {
	var p = PeptideRuntime.data.effects.get(AS3.str(id));
	return p == null || p.length == 0 ? "active" : p[0].anim;
};
