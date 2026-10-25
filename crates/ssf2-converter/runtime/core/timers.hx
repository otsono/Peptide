// The source's timers are the engine's own. `createTimer(frames, repeat, fn)` runs `fn` every
// `frames` of the source's frames, `repeat` times (0 is forever): the engine's timer of twice the
// frames (a source frame is two engine frames), whose forever is -1. a fighter's timer ends with
// the state it was started in unless asked to persist, as the engine's own does; any other
// object's lasts as long as the object. an object keeps one timer per function: starting it again
// restarts it, and the source removes it by naming the function.

var Timers = {};

Timers.create = function(owner, frames, repeat, fn, ?opts, ?fromFrame) {
	if (fn == null) return;
	var fm = owner == null || owner.__fm == null ? PeptideRuntime.fm : owner.__fm;
	if (fm == null || fm.isDisposed()) return;
	var key = owner == null ? PeptideRuntime.object : owner;
	Timers.destroy(key, fn);
	var n = Std.int(AS3.num(frames));
	if (n < 1) n = 1;
	var r = Std.int(AS3.num(repeat));
	if (r <= 0) r = -1;
	// a fighter's timers drop with its state unless asked to stay; anything else's stay (the stage
	// is no game object and has no type to ask)
	var keep = !PeptideRuntime.isFighter(fm) || (opts != null && AS3.truthy(opts.persistent));
	var fighterKey = !keep && key.__fm != null && !key.__fm.isDisposed();
	var rec = { fn: fn, fm: fm, id: -1, keep: keep, entry: fighterKey ? Events.entry(key) : -1 };
	// one made in a frame's script (the compiler says so) comes due two engine frames sooner: the
	// source ticks the frame it is made in too, ahead of that frame's movement (captain falcon's
	// kick makes a one-frame timer on frame 7 and moves on its speed for the whole of that frame).
	// not one run as its object is made, before its clips are placed (waluigi's dice spinner)
	var first = fromFrame == true && PeptideRuntime.making != true ? 2 * n - 2 : 2 * n;
	// a timer made with a condition (`condition`, a function) runs its code only while that holds:
	// chibi-robo's timer fires his telly whenever he has none, and fired a new one every frame here
	// (the last ended, the next made at the spawn, never moving off it)
	var cond = opts == null ? null : opts.condition;
	var fire = function() {
		var was = PeptideRuntime.inTimer;
		PeptideRuntime.inTimer = true;
		if (cond == null || AS3.truthy(PeptideRuntime.run(cond))) PeptideRuntime.run(fn);
		PeptideRuntime.inTimer = was;
	};
	// the frames its object stood frozen by a hit do not count toward it: the source runs none of
	// that object while it is held still, its timers included (pac-man's side special refreshes on a
	// two-frame timer and hit every three frames there, a frame of freeze after each hit; every two
	// here). counted by the object's own runtime (PeptideRuntime.clockedUpdate); a timer that comes
	// due owing frames waits them out first
	// (kept on the record: a script's local function cannot call itself by its own name)
	var frozenOf = function() { return key.__frozenFrames == null ? 0 : key.__frozenFrames; };
	rec.armed = frozenOf();
	rec.due = function(then) {
		var owed = frozenOf() - rec.armed;
		rec.armed = frozenOf();
		if (owed > 0) {
			rec.id = fm.addTimer(owed, 1, function(?t) { if (!fm.isDisposed() && rec.dead != true) rec.due(then); }, { persistent: keep });
			return;
		}
		then();
	};
	// the timer that fires `left` more times (-1 forever) every whole period, each period armed as
	// the last one fires (so the frames it owes push the rest back too); a timer that has fired its
	// last is gone from the engine, and its record goes too
	rec.repeat = function(delay, left) {
		rec.id = fm.addTimer(delay, 1, function(?t) {
			if (fm.isDisposed()) return;
			rec.due(function() {
				if (left == 1) Timers.drop(key, rec); else { rec.armed = frozenOf(); rec.repeat(2 * n, left < 0 ? -1 : left - 1); }
				fire();
			});
		}, { persistent: keep });
	};
	var repeat = rec.repeat;
	if (key.__timers == null) key.__timers = [];
	// a fighter's timer the engine let go with the state it was made in is gone: its record goes
	// too, or the list grows with every attack the fighter makes (mario's grew by eighteen in
	// twenty seconds of fighting)
	if (fighterKey && key.__timers.length > 0) {
		var now = rec.entry;
		var kept = [];
		for (t in key.__timers) { if (t.keep == false && t.entry != now) t.dead = true; else kept.push(t); }
		key.__timers = kept;
	}
	key.__timers.push(rec);
	// a one-frame one made in a frame's script ticks in that frame, once the script is done:
	// sooner, it reads what the script sets after making it (samus's moves); later, every tick
	// after moves with it (captain falcon's kick ran three source frames short)
	if (first == 0) {
		if (r != 1) repeat(2 * n, r < 0 ? -1 : r - 1); else Timers.drop(key, rec);
		PeptideRuntime.afterFrame.push(function() { if (!fm.isDisposed() && rec.dead != true) fire(); });
	} else if (first == 2 * n || r == 1) {
		repeat(first, r);
	} else {
		rec.id = fm.addTimer(first, 1, function(?t) {
			if (fm.isDisposed()) return;
			rec.due(function() {
				rec.armed = frozenOf();
				rec.repeat(2 * n, r < 0 ? -1 : r - 1);
				fire();
			});
		}, { persistent: keep });
	}
};
Timers.drop = function(owner, rec) {
	if (owner.__timers != null) owner.__timers.remove(rec);
};

Timers.destroy = function(owner, fn) {
	if (owner == null) owner = PeptideRuntime.object;
	if (owner == null || owner.__timers == null) return;
	for (t in owner.__timers.copy()) {
		if (t.fn != fn) continue;
		if (!t.fm.isDisposed()) t.fm.removeTimer(t.id);
		t.dead = true;
		owner.__timers.remove(t);
	}
};
