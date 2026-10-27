// Flash's display objects, as the source's code sees them: clips with timelines, playheads, labels
// and frame scripts, and the children each frame places. This is logic only. What the player sees
// is the engine's entity; the runtime shows it the frame these playheads are on (see runtime.hx).
//
// A clip's timeline comes from the data section (`DATA.timelines`, by symbol): for each frame, the
// children it places that logic can see (those with a class or a name), and its labels. A named
// child is bound to the field of the same name, as Flash binds a placed instance.

// class DisplayObject
var DisplayObject = { name: "DisplayObject" };
DisplayObject.unnamed = 0;
DisplayObject.build = function(me) {
	if (me.__is == null) me.__is = [];
	me.__is.push("DisplayObject");
	// one placed under no name is `instanceN`, as Flash names it (`__auto`: no name was placed)
	DisplayObject.unnamed++;
	me.name = "instance" + DisplayObject.unnamed;
	me.__auto = true;
	me.parent = null;
	me.x = 0;
	me.y = 0;
	me.visible = true;
	me.alpha = 1;
	me.rotation = 0;
	me.scaleX = 1;
	me.scaleY = 1;
	me.transform = Transform.create(me);
	// a point between this clip's space and the stage's, through its parents' places and scales
	// (a clip of an object is at the object)
	me.localToGlobal = function(p) {
		var x = AS3.num(p.x);
		var y = AS3.num(p.y);
		var c = me;
		var n = 0;
		while (c != null && n < 64) {
			x = x * AS3.num(c.scaleX) + AS3.num(c.x);
			y = y * AS3.num(c.scaleY) + AS3.num(c.y);
			// the stage's clip is the stage's space already: the source's own places
			if (c.__object != null && c.__object.__type == "SSF2Stage") break;
			if (c.__object != null) { x = c.__object.__fm.getX() / PeptideRuntime.data.units.size + c.__object.__fm.flipX(x); y = c.__object.__fm.getY() / PeptideRuntime.data.units.size + y; break; }
			c = c.parent;
			n++;
		}
		return Point.create(x, y);
	};
	// (a method the package's code never names is not made: DATA.clipUses, none stated = all)
	var uses = PeptideRuntime.data == null ? null : PeptideRuntime.data.clipUses;
	if (uses == null || uses.globalToLocal == true) me.globalToLocal = function(p) {
		var o = me.localToGlobal(Point.create(0, 0));
		return Point.create(AS3.num(p.x) - o.x, AS3.num(p.y) - o.y);
	};
	// the topmost clip up its parents
	me.get_root = function() {
		var c = me;
		var n = 0;
		while (c.parent != null && n < 64) { c = c.parent; n++; }
		return c;
	};
};
DisplayObject.init = function(me) {};
DisplayObject.create = function() { var me = {}; DisplayObject.build(me); return me; };

// a clip's vector drawing: this engine draws no shapes as the code runs, so drawing is a gap; what
// is drawn is kept as its extent (`__vec`, the clip's own space), which a surface made from the
// clip reads. each clip's is made the first time its code asks for it
// class Graphics
var Graphics = { name: "Graphics" };
Graphics.build = function(me, clip) {
	var grow = function(x0, y0, x1, y1) {
		var v = clip.__vec;
		if (v == null) { clip.__vec = { x0: x0, y0: y0, x1: x1, y1: y1 }; return; }
		v.x0 = Math.min(v.x0, x0); v.y0 = Math.min(v.y0, y0); v.x1 = Math.max(v.x1, x1); v.y1 = Math.max(v.y1, y1);
	};
	me.beginFill = function(?c, ?a) { PeptideRuntime.gap("graphics: vector drawing as the code runs"); };
	me.beginBitmapFill = function(?b, ?m, ?r, ?s) { PeptideRuntime.gap("graphics: vector drawing as the code runs"); };
	me.lineStyle = function(?t, ?c, ?a, ?h, ?s, ?cp, ?j, ?m) {};
	me.moveTo = function(x, y) { grow(AS3.num(x), AS3.num(y), AS3.num(x), AS3.num(y)); };
	me.lineTo = function(x, y) { grow(AS3.num(x), AS3.num(y), AS3.num(x), AS3.num(y)); };
	me.curveTo = function(a, b, c, d) { grow(AS3.num(c), AS3.num(d), AS3.num(c), AS3.num(d)); };
	me.drawRect = function(x, y, w, h) { var a = AS3.num(x); var b = AS3.num(y); grow(Math.min(a, a + AS3.num(w)), Math.min(b, b + AS3.num(h)), Math.max(a, a + AS3.num(w)), Math.max(b, b + AS3.num(h))); };
	me.drawRoundRect = function(x, y, w, h, ew, ?eh) { me.drawRect(x, y, w, h); };
	me.drawCircle = function(x, y, r) { var c = AS3.num(x); var d = AS3.num(y); var q = Math.abs(AS3.num(r)); grow(c - q, d - q, c + q, d + q); };
	me.drawEllipse = function(x, y, w, h) { me.drawRect(x, y, w, h); };
	me.endFill = function() {};
	me.clear = function() { clip.__vec = null; };
};

// class MovieClip extends DisplayObject
var MovieClip = { name: "MovieClip" };
MovieClip.build = function(me) {
	DisplayObject.build(me);
	me.__is.push("Sprite");
	me.__is.push("MovieClip");
	me.__sym = -1;
	var uses = PeptideRuntime.data == null ? null : PeptideRuntime.data.clipUses;
	var all = uses == null;
	if (all || uses.get_graphics == true) me.get_graphics = function() { if (me.__graphics == null) { me.__graphics = {}; Graphics.build(me.__graphics, me); } return me.__graphics; };
	me.__tl = null;
	me.__playing = true;
	me.__enteredOn = -1;
	me.__scripts = new haxe.ds.IntMap();
	me.__kids = [];
	me.currentFrame = 1;
	me.currentLabel = null;
	// the label on the frame the playhead is on itself (none between labels)
	me.currentFrameLabel = null;
	me.totalFrames = 1;
	// addFrameScript(frame, script, frame, script, ...): the scripts a class gives its frames (0-based)
	me.addFrameScript = function(rest) {
		var i = 0;
		while (i + 1 < rest.length) {
			if (rest[i + 1] == null) me.__scripts.remove(rest[i]); else me.__scripts.set(rest[i], rest[i + 1]);
			i += 2;
		}
	};
	// the playhead, moved by any script: a clip whose script has ended is gone, and moving it does
	// nothing (checked through the local, which needs none of that script's names)
	var life = PeptideRuntime.life;
	me.play = function() { if (life.on != true) return; me.__playing = true; PeptideRuntime.hold(me, false); };
	me.stop = function() { if (life.on != true) return; me.__playing = false; PeptideRuntime.hold(me, true); };
	me.gotoAndPlay = function(frame, ?scene) { if (life.on != true) return; me.__playing = true; MovieClip.goto(me, frame); };
	me.gotoAndStop = function(frame, ?scene) { if (life.on != true) return; me.__playing = false; MovieClip.goto(me, frame); };
	if (all || uses.nextFrame == true) me.nextFrame = function() { if (life.on != true) return; me.__playing = false; MovieClip.goto(me, me.currentFrame + 1); };
	if (all || uses.prevFrame == true) me.prevFrame = function() { if (life.on != true) return; me.__playing = false; MovieClip.goto(me, me.currentFrame - 1); };
	me.getChildByName = function(n) {
		for (k in me.__kids) if (k.clip.name == n) return k.clip;
		return null;
	};
	me.addChild = function(c) {
		if (c.parent != null && c.parent.removeChild != null) c.parent.removeChild(c);
		c.parent = me;
		me.__kids.push({ depth: -1, sym: -1, clip: c });
		return c;
	};
	if (all || uses.addChildAt == true) me.addChildAt = function(c, n) {
		if (c.parent != null && c.parent.removeChild != null) c.parent.removeChild(c);
		c.parent = me;
		var at = AS3.int(n);
		if (at < 0) at = 0;
		if (at > me.__kids.length) at = me.__kids.length;
		me.__kids.insert(at, { depth: -1, sym: -1, clip: c });
		return c;
	};
	me.removeChild = function(c) {
		var i = 0;
		while (i < me.__kids.length) { if (me.__kids[i].clip == c) { me.__kids.splice(i, 1); break; } i++; }
		c.parent = null;
		return c;
	};
	if (all || uses.removeChildren == true) me.removeChildren = function(?from, ?to) {
		var a = from == null ? 0 : AS3.int(from);
		var b = to == null ? me.__kids.length - 1 : AS3.int(to);
		var gone = [];
		for (i in a...(b + 1)) if (i < me.__kids.length) gone.push(me.__kids[i].clip);
		for (c in gone) me.removeChild(c);
	};
	if (all || uses.get_numChildren == true) me.get_numChildren = function() { return me.__kids.length; };
	// its box in another clip's space (its parent's when none is given): the art's own extent,
	// placed and scaled by each clip on the way up (a turn is not followed)
	me.getRect = function(?space) {
		var bs = PeptideRuntime.data.symBounds;
		var b = bs == null ? null : bs.get(me.__sym);
		if (b == null) return Rectangle.create(me.x, me.y, 0, 0);
		var x = b.x; var y = b.y; var w = b.w; var h = b.h;
		var c = me;
		var n = 0;
		while (c != null && c != space && n < 64) {
			var sx = c.scaleX == null ? 1 : c.scaleX;
			var sy = c.scaleY == null ? 1 : c.scaleY;
			x = c.x + x * sx; y = c.y + y * sy; w = w * sx; h = h * sy;
			if (space == null) break;
			c = c.parent;
			n++;
		}
		if (w < 0) { x = x + w; w = -w; }
		if (h < 0) { y = y + h; h = -h; }
		return Rectangle.create(x, y, w, h);
	};
	me.getBounds = me.getRect;
	me.getChildIndex = function(c) {
		var i = 0;
		while (i < me.__kids.length) { if (me.__kids[i].clip == c) return i; i++; }
		return -1;
	};
	if (all || uses.setChildIndex == true) me.setChildIndex = function(c, n) {
		if (life.on != true) return;
		var i = me.getChildIndex(c);
		if (i < 0) return;
		var k = me.__kids[i];
		me.__kids.splice(i, 1);
		var at = AS3.int(n);
		if (at > me.__kids.length) at = me.__kids.length;
		if (at < 0) at = 0;
		me.__kids.insert(at, k);
	};
	// two children trade places in the drawing order
	if (all || uses.swapChildren == true) me.swapChildren = function(a, b) {
		var i = -1;
		var j = -1;
		var n = 0;
		while (n < me.__kids.length) { if (me.__kids[n].clip == a) i = n; if (me.__kids[n].clip == b) j = n; n++; }
		if (i < 0 || j < 0) return;
		var t = me.__kids[i];
		me.__kids[i] = me.__kids[j];
		me.__kids[j] = t;
	};
	if (all || uses.getChildAt == true) me.getChildAt = function(i) { return (i >= 0 && i < me.__kids.length) ? me.__kids[i].clip : null; };
};
// a clip class the code constructs (`new k1_HUD()`) has its symbol's timeline, as one the tree
// places does: the children its first frame places are there once it is made (kingdom1 writes
// into its HUD's text field). the frame's script runs where a placement enters it
MovieClip.init = function(me) {
	if (me.__sym != -1 || me.__is == null || me.__is.length == 0) return;
	var sym = MovieClip.symOf(me.__is[me.__is.length - 1]);
	if (sym == null) return;
	MovieClip.attach(me, sym, true);
	if (me.__tl != null) MovieClip.arrive(me, 1);
};
MovieClip.syms = null;
MovieClip.symOf = function(name) {
	var d = PeptideRuntime.data;
	if (d == null || d.classes == null) return null;
	if (MovieClip.syms == null) {
		MovieClip.syms = new haxe.ds.StringMap();
		for (k in d.classes.keys()) { var c = d.classes.get(k); if (c != null && c.name != null && !MovieClip.syms.exists(c.name)) MovieClip.syms.set(c.name, k); }
	}
	return MovieClip.syms.get(name);
};
MovieClip.create = function() { var me = {}; MovieClip.build(me); return me; };

// give a clip the timeline its symbol has, and put it on its first frame
MovieClip.attach = function(me, sym, ?later) {
	me.__sym = sym;
	me.__tl = PeptideRuntime.data.timelines.get(sym);
	me.totalFrames = me.__tl == null ? 1 : me.__tl.frames.length;
	me.currentFrame = 0;
	if (later != true) MovieClip.enter(me, 1);
};

// the frame a number or a label names (a label it does not have is no frame)
MovieClip.frameOf = function(me, f) {
	if (Std.isOfType(f, String)) {
		if (me.__tl == null) return -1;
		for (l in me.__tl.labels) if (l[1] == f) return l[0];
		return -1;
	}
	return Std.int(f);
};

// a jump to the frame the playhead is already on changes nothing (its script does not run again).
// a step between frames (code that moves the playhead on each frame, run at half its step) is
// kept, and the playhead moves once the steps make a whole frame
MovieClip.goto = function(me, f) {
	if (!Std.isOfType(f, String) && AS3.num(f) % 1 != 0) {
		var at = AS3.num(f) + (me.__part == null ? 0 : me.__part);
		me.__part = at - Math.floor(at);
		f = Math.floor(at);
	} else me.__part = 0;
	var n = MovieClip.frameOf(me, f);
	if (n < 1) return;
	if (n > me.totalFrames) n = me.totalFrames;
	if (n == me.currentFrame) return;
	// the clip an engine animation shows jumped: the engine is moved to the frame that shows it.
	// said before the frame's script runs, as the playhead's move stands whatever that script does:
	// one that stops on an error (saffron city's charmander reads a flame it never fired) left
	// the engine where it was, the clip was put back there, and the code jumped it again every frame
	if (PeptideRuntime.native && me == PeptideRuntime.shownClip() && PeptideRuntime.engineJump(me, n)) return;
	// the root sent to a label its code names: the engine plays the animation that label's clip
	// shows, whose first keyframe puts the root there and runs the clip's first frame
	if (PeptideRuntime.native && me == PeptideRuntime.root && !PeptideRuntime.switching && Std.isOfType(f, String)
		&& PeptideRuntime.engineLabel(AS3.str(f))) return;
	if (PeptideRuntime.native && me.__sym != null && PeptideRuntime.data.nativeSyms != null && PeptideRuntime.data.nativeSyms.exists(me.__sym)
		&& PeptideRuntime.shows(me.__sym, n)) { MovieClip.arrive(me, n); return; }
	PeptideRuntime.moved(me);
	MovieClip.enter(me, n);
};

// the playhead enters a frame: its children are placed, its label is read, its script runs
MovieClip.enter = function(me, n) {
	MovieClip.arrive(me, n);
	// entered while its object is being made, the frame's script waits for the object's first
	// update (PeptideRuntime.held)
	if (PeptideRuntime.making && PeptideRuntime.held != null) { me.__held = { clip: me, frame: n }; PeptideRuntime.held.push(me.__held); return; }
	MovieClip.runFrame(me, n);
};

// the playhead is on a frame: its children placed and its label read, its script not run (the
// engine runs it, from the frame's keyframe)
MovieClip.arrive = function(me, n) {
	var from = me.currentFrame;
	me.currentFrame = n;
	var tl = me.__tl;
	if (tl != null) {
		// the label each frame is under, and the one on it, worked out once a timeline (arriving is
		// every other frame of every fighter, and a timeline's labels never change)
		if (tl.__labelAt == null) {
			var under = [];
			var on = [];
			var f = 1;
			while (f <= tl.frames.length) {
				var u = null;
				var o = null;
				for (l in tl.labels) {
					if (l[0] <= f) u = l[1];
					if (l[0] == f) o = l[1];
				}
				under.push(u);
				on.push(o);
				f++;
			}
			tl.__labelOn = on;
			tl.__labelAt = under;
		}
		var i = Std.int(n) - 1;
		var inside = i >= 0 && i < tl.__labelAt.length;
		me.currentLabel = inside ? tl.__labelAt[i] : null;
		me.currentFrameLabel = inside ? tl.__labelOn[i] : null;
		if (!inside) for (l in tl.labels) {
			if (l[0] <= n) me.currentLabel = l[1];
			if (l[0] == n) me.currentFrameLabel = l[1];
		}
		// a step on to a frame that places what the one before did leaves the children as they are
		// (as Flash does: a child the code took off stays off)
		if (!(from == n - 1 && MovieClip.sameAsBefore(tl, i))) MovieClip.place(me, tl.frames[n - 1]);
	}
};

// whether a frame places just what the frame before it does, worked out once a timeline
MovieClip.sameAsBefore = function(tl, i) {
	if (tl.__same == null) {
		var same = [false];
		var f = 1;
		while (f < tl.frames.length) { same.push(MovieClip.samePlacing(tl.frames[f - 1], tl.frames[f])); f++; }
		tl.__same = same;
	}
	return i > 0 && i < tl.__same.length && tl.__same[i];
};
MovieClip.samePlacing = function(a, b) {
	if (a == b) return true;
	if (a == null || b == null || a.length != b.length) return false;
	var k = 0;
	while (k < a.length) {
		var x = a[k];
		var y = b[k];
		if (x.length != y.length || x[0] != y[0] || x[1] != y[1] || x[2] != y[2]) return false;
		var at = x.length > 3 ? x[3] : null;
		var bt = y.length > 3 ? y[3] : null;
		if (at != bt) {
			if (at == null || bt == null || at.length != bt.length) return false;
			var j = 0;
			while (j < at.length) { if (at[j] != bt[j]) return false; j++; }
		}
		k++;
	}
	return true;
};

MovieClip.runFrame = function(me, n) {
	// a clip the engine plays as the object's own runs the frames its animations show from the
	// entity's keyframes (a frame none shows runs here, when code sends the clip to it)
	if (PeptideRuntime.native && PeptideRuntime.data.nativeSyms != null && PeptideRuntime.data.nativeSyms.exists(me.__sym)
		&& PeptideRuntime.shows(me.__sym, n)) return;
	var s = me.__scripts.get(n - 1);
	if (s != null) PeptideRuntime.run(s);
};

// the children a frame places: kept where the same symbol stays at the same depth, made anew
// where it changes, taken off where it goes
MovieClip.place = function(me, want) {
	var keep = [];
	for (k in me.__kids) {
		if (k.depth < 0) { keep.push(k); continue; }
		var stays = false;
		for (w in want) if (w[0] == k.depth && w[1] == k.sym) stays = true;
		if (stays) keep.push(k); else {
			k.clip.parent = null;
			PeptideRuntime.release(k.clip);
			if (k.clip.__auto != true && me.__tl.bind != null) me.__tl.bind(me, k.clip.name, null);
		}
	}
	me.__kids = keep;
	for (w in want) {
		var at = w.length > 3 ? w[3] : null;
		var have = null;
		for (k in me.__kids) if (k.depth == w[0] && k.sym == w[1]) have = k;
		if (have != null) {
			if (have.clip != null && (at != null || have.clip.__tlAt != null)) MovieClip.put(have.clip, at);
			continue;
		}
		var kid = { depth: w[0], sym: w[1], clip: null };
		me.__kids.push(kid);
		var bind = (w[2] != null && me.__tl.bind != null)
			? function(c) { kid.clip = c; if (at != null) MovieClip.put(c, at); me.__tl.bind(me, w[2], c); }
			: function(c) { kid.clip = c; if (at != null) MovieClip.put(c, at); };
		PeptideRuntime.makeClip(w[1], me, w[2], bind);
	}
};

// where the frame puts a child (x, y, scale x and y, rotation), until the code sets one of them:
// from then on the child is the code's, as Flash hands a placed child over
MovieClip.put = function(c, at) {
	var was = c.__tlAt;
	if (was != null && (c.x != was[0] || c.y != was[1] || c.scaleX != was[2] || c.scaleY != was[3] || c.rotation != was[4])) return;
	var v = at == null ? [0.0, 0.0, 1.0, 1.0, 0.0] : at;
	c.x = v[0]; c.y = v[1]; c.scaleX = v[2]; c.scaleY = v[3]; c.rotation = v[4];
	c.__tlAt = [c.x, c.y, c.scaleX, c.scaleY, c.rotation];
};
// how far the code has moved a child off where its frame puts it
MovieClip.movedX = function(c) { return AS3.num(c.x) - (c.__tlAt == null ? 0.0 : AS3.num(c.__tlAt[0])); };
MovieClip.movedY = function(c) { return AS3.num(c.y) - (c.__tlAt == null ? 0.0 : AS3.num(c.__tlAt[1])); };

// once a source frame: the playhead moves on (round to the first frame at the end, as Flash does)
// unless it is stopped; then the children that were already there do the same
MovieClip.tick = function(me) {
	// put on its frame by the engine's state entry this same frame: it is already where the
	// source's frame shows it
	if (me.__enteredOn == PeptideRuntime.frame) return;
	// (the children as they were before this frame moved it; nothing copied when there are none)
	var before = me.__kids.length == 0 ? null : me.__kids.copy();
	// the clip an engine animation shows moves with the engine's frame (PeptideRuntime.sync); its
	// children still move here. asked only of a clip that would move
	if (me.__playing && me.totalFrames > 1 && !PeptideRuntime.clocked(me)) {
		MovieClip.enter(me, me.currentFrame >= me.totalFrames ? 1 : me.currentFrame + 1);
	}
	// (a child of one frame with none of its own has nothing to move: not visited)
	if (before != null) for (k in before) {
		var c = k.clip;
		if (c.__tl != null && c.parent == me && (c.totalFrames > 1 || c.__kids.length > 0)) MovieClip.tick(c);
	}
};
