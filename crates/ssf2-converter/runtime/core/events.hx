// Listeners the source's code puts on an object, by the source's event names. An event this
// engine raises itself (`runtime/events.jsonc`) is the engine's own listener on the object's
// handle, handed the event as the source's record. A change of state is the engine's too, told
// with the source's states either side. The rest the runtime raises where the source's engine
// would (the end of an attack, a projectile's end).

var Events = {};
// the objects listening for `enterFrame`: told once every source frame, as Flash tells every
// display object
Events.everyFrame = [];
// a listener on a fighter lasts until the fighter's state next changes unless it is added to
// persist, as the engine's own do (measured: ganondorf's side special leaves a landing listener
// behind that the source never runs again; kept, it landed his down special on its next touch
// of the floor). on anything else it lasts until it is taken off
Events.add = function(owner, type, fn, ?opts) {
	if (owner == null) return;
	var fm = owner.__fm;
	var persistent = opts != null && AS3.truthy(AS3.at(opts, "persistent"));
	// an event the engine raises: the engine's own listener, for this one (the conversion writes
	// the same for a listener the source's code adds for a named event)
	var ev = fm == null ? null : PeptideRuntime.data.engineEvent(AS3.str(type));
	if (ev != null) {
		var h = Events.handler(owner, type, fn);
		fm.removeEventListener(ev, h);
		fm.addEventListener(ev, h, { persistent: persistent || !PeptideRuntime.isFighter(fm) });
		return;
	}
	if (owner.__listeners == null) owner.__listeners = [];
	Events.remove(owner, type, fn);
	if (type == "enterFrame" && Events.everyFrame.indexOf(owner) < 0) Events.everyFrame.push(owner);
	var fighter = fm != null && !fm.isDisposed() && PeptideRuntime.isFighter(fm);
	var keep = !fighter || persistent;
	// by the script that added it: that script's names answer it, so it is called only while that
	// script lives
	owner.__listeners.push({ type: type, fn: fn, life: PeptideRuntime.life, keep: keep, entry: fighter ? Events.entry(owner) : 0 });
	Events.wire(owner, AS3.str(type));
};
// the engine's listener for one of the source's: the same function each time for the same
// object, event and listener, so one added is taken off again. it hands the listener the event
// as the source's record, while the script that added it lives
Events.handler = function(owner, type, fn) {
	if (owner.__handlers == null) owner.__handlers = [];
	for (x in owner.__handlers) if (x.type == type && x.fn == fn) return x.h;
	var life = PeptideRuntime.life;
	var fm = owner.__fm;
	var engineEv = PeptideRuntime.data.engineEvent(AS3.str(type));
	var floor = engineEv == EntityEvent.COLLIDE_FLOOR;
	var hit = engineEv == GameObjectEvent.HIT_DEALT || engineEv == GameObjectEvent.HIT_RECEIVED;
	var h = function(?e) {
		if ((life != null && life.on != true) || fm.isDisposed()) return;
		// a projectile made on the floor is told of it with the speed it was launched at, which
		// the engine's floor has already taken off it: the source tells the touch first (pikachu's
		// jolt turns its launch, down and ahead, into its first hop up and ahead)
		var l = floor && fm.exports != null ? fm.exports.__launched : null;
		if (l != null) fm.exports.__launched = null;
		// a touch where it was made, short of where it is fired to, is the engine's making of it at
		// its owner's feet, not a landing (waluigi's dice, thrown from above them, spent its one
		// bounce there)
		if (l != null && l.y != null && Math.abs(fm.getY() - l.y) > 1) return;
		// any other is told with the speed it came down at (waluigi's dice and krystal's grenade
		// bounce off it once, by their own code)
		else if (floor && fm.exports != null && fm.exports.__airSpeed != null) { l = fm.exports.__airSpeed; fm.exports.__airSpeed = null; }
		var given = l != null && l.xs != null && fm.getYSpeed() == 0 && l.ys != 0;
		var cx = fm.getXSpeed();
		if (given) { fm.setXSpeed(l.xs); fm.setYSpeed(l.ys); }
		// the source has put the fighter a hit stuns in its hurt state by the time it tells of the
		// hit, where this engine turns it after its hit events (krystal's jab asks whether the one
		// it struck is hurt before it pulls it in)
		var struck = !hit ? null : engineEv == GameObjectEvent.HIT_DEALT ? (e == null || e.data == null ? null : e.data.foe) : fm;
		if (struck != null && (struck.getType() != EntityType.CHARACTER || !(struck.getHitstun() > 0) || struck.exports == null)) struck = null;
		if (struck != null) struck.exports.__struck = true;
		PeptideRuntime.run(fn, { type: type, target: owner, data: Events.record(owner, e) });
		if (struck != null) struck.exports.__struck = null;
		if (given && !fm.isDisposed() && fm.getXSpeed() == l.xs && fm.getYSpeed() == l.ys) { fm.setXSpeed(cx); fm.setYSpeed(0); }
		// a speed its code gave it off the floor is the engine's to keep once its landing is done:
		// the engine settles the touch after telling of it, and the bounce went with it
		else if (floor && !fm.isDisposed() && fm.exports != null && fm.getYSpeed() < 0) fm.exports.__floorSet = { xs: fm.getXSpeed(), ys: fm.getYSpeed() };
	};
	owner.__handlers.push({ type: type, fn: fn, h: h });
	// the handlers kept so one added is the one taken off: frame scripts make new listeners each
	// time they run, and the list grew with every attack. past a few dozen, one the engine no
	// longer holds is let go (taking it off or adding it again makes a new one, which is the same)
	if (owner.__handlers.length > 32 && fm != null && !fm.isDisposed()) {
		var kept = [];
		for (x in owner.__handlers) {
			var ev = PeptideRuntime.data.engineEvent(AS3.str(x.type));
			if (x.h == h || ev == null || fm.hasEventListener(ev, x.h)) kept.push(x);
		}
		owner.__handlers = kept;
	}
	return h;
};
// tell an object's listeners for the engine's floor contact, as the engine would on a touch:
// false when it has none yet
Events.touchedFloor = function(owner) {
	if (owner.__handlers == null) return false;
	var told = false;
	for (x in owner.__handlers.copy()) {
		if (PeptideRuntime.data.engineEvent(AS3.str(x.type)) != EntityEvent.COLLIDE_FLOOR) continue;
		told = true;
		x.h(null);
	}
	return told;
};
Events.handled = function(owner, type, ?fn) {
	if (owner.__handlers == null) return [];
	var out = [];
	for (x in owner.__handlers) if (x.type == type && (fn == null || x.fn == fn)) out.push(x);
	return out;
};
// which of its states a fighter is in, counted: a new count each time its state changes
Events.entry = function(owner) {
	var st = owner.__fm.getState();
	if (st != owner.__lastState) {
		owner.__lastState = st;
		owner.__entry = (owner.__entry == null ? 0 : owner.__entry) + 1;
	}
	return owner.__entry;
};
// every update of a fighter: the listeners of a state it has left go (the change itself was told
// to them first, as the engine raises it ahead of the update)
Events.stateMoved = function(owner) {
	if (owner.__listeners == null) return;
	var now = Events.entry(owner);
	// nothing has left a state since the last look (most frames): the listeners stand
	if (now == owner.__movedAt && owner.__listeners.length == owner.__movedCount) return;
	var gone = false;
	for (l in owner.__listeners) if (l.keep != true && l.entry != now) { gone = true; break; }
	if (!gone) { owner.__movedAt = now; owner.__movedCount = owner.__listeners.length; return; }
	var kept = [];
	for (l in owner.__listeners) if (l.keep == true || l.entry == now) kept.push(l);
	owner.__listeners = kept;
	owner.__movedAt = now;
	owner.__movedCount = kept.length;
	if (!Events.has(owner, "enterFrame")) Events.everyFrame.remove(owner);
};
Events.remove = function(owner, type, fn) {
	if (owner == null) return;
	var fm = owner.__fm;
	var ev = fm == null ? null : PeptideRuntime.data.engineEvent(AS3.str(type));
	if (ev != null) {
		for (x in Events.handled(owner, type, fn)) { fm.removeEventListener(ev, x.h); owner.__handlers.remove(x); }
		return;
	}
	if (owner.__listeners == null) return;
	var keep = [];
	for (l in owner.__listeners) if (!(l.type == type && l.fn == fn)) keep.push(l);
	owner.__listeners = keep;
	if (type == "enterFrame" && !Events.has(owner, type)) Events.everyFrame.remove(owner);
};
Events.has = function(owner, type, ?fn) {
	if (owner == null) return false;
	var fm = owner.__fm;
	var ev = fm == null ? null : PeptideRuntime.data.engineEvent(AS3.str(type));
	if (ev != null) {
		for (x in Events.handled(owner, type, fn)) if (fm.hasEventListener(ev, x.h)) return true;
		return false;
	}
	if (owner.__listeners == null) return false;
	for (l in owner.__listeners) if (l.type == type && (fn == null || l.fn == fn)) return true;
	return false;
};
// tell an object's listeners an event of `type` happened
// (an event raised again from inside its own listeners, deeper than any source's chain of
// them goes, is a loop: reported, not followed down)
Events.depth = 0;
// (`life`: only the listeners one script added, for an event that script's own wiring heard)
Events.dispatch = function(owner, type, ?data, ?life) {
	if (owner == null || owner.__listeners == null) return;
	// most events most frames have no one listening: nothing is made for them
	var heard = false;
	for (l in owner.__listeners) if (l.type == type) { heard = true; break; }
	if (!heard) return;
	if (Events.depth > 16) { PeptideRuntime.report("the event " + type + " raised itself over and over"); return; }
	var e = { type: type, target: owner, data: data == null ? {} : data };
	Events.depth++;
	for (l in owner.__listeners.copy()) {
		if (l.type != type || (life != null && l.life != life)) continue;
		if (l.life != null && l.life.on != true) continue;
		PeptideRuntime.run(l.fn, e);
	}
	Events.depth--;
};

Events.wire = function(owner, type) {
	var fm = owner.__fm;
	if (fm == null) return;
	var ev = PeptideRuntime.data.stateEvent(type);
	if (ev == null) return;
	// once per object, event and script: each script hears the engine's event for the listeners it
	// added, while it lives. the local `life` needs none of the script's names, so an event after
	// the script ended is dropped before anything of it is asked
	var life = PeptideRuntime.life;
	if (owner.__wired == null) owner.__wired = new haxe.ds.StringMap();
	var lives = owner.__wired.get(type);
	if (lives == null) { lives = []; owner.__wired.set(type, lives); }
	if (lives.indexOf(life) >= 0) return;
	lives.push(life);
	if (owner.__srcState == null) owner.__srcState = States.of(fm);
	var last = owner.__srcState;
	fm.addEventListener(ev, function(?e) {
		if (life.on != true || fm.isDisposed()) return;
		var to = fm.getType() == EntityType.CHARACTER ? States.of(fm) : fm.getState();
		if (to == last) return;
		var from = last;
		last = to;
		Events.dispatch(owner, type, { fromState: from, toState: to, caller: owner }, life);
	}, { persistent: true });
};

// what the engine's event says, as the source's record: whoever is on the other end under every
// name the source uses for it, and the box's stats as the source's
Events.record = function(owner, e) {
	var d = e == null ? null : e.data;
	var foe = d == null ? null : Objects.held(d.foe);
	var box = d == null ? null : Hitboxes.fromEngine(d.hitboxStats);
	return { receiver: foe, opponent: foe, grabbed: foe, grabber: foe, projectile: foe, caller: owner, attackBoxData: box };
};
