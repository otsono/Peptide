// The entry the engine calls on a fighter's script. Emitted last, after the fighter's data.

function initialize() {
	PeptideRuntime.start(self, DATA, Fighters);
	self.exports.__verifyPulse = __verifyPulse;
	// a jump by label asked by another object's code (a projectile written as Fraymakers code jumps
	// its owner's clip through this), run as the fighter's own code
	self.exports.__playFrame = function(label) { PeptideRuntime.run(function() { PeptideGameObject.playFrame(PeptideRuntime.object, label); }); };
	// an attack another object's code starts on it (at a frame of its clip), and a rocket it rides
	self.exports.__forceAttack = function(label, ?frame) { PeptideRuntime.run(function() { Fighters.startAsked(PeptideRuntime.object, label, frame); }); };
	self.exports.__rocket = function(speed, angle, decay, ?rotate) { PeptideRuntime.run(function() { PeptideCharacter.rocket(PeptideRuntime.object, speed, angle, decay, rotate); }); };
	// the source's label its clip is on, for another object's code
	self.exports.__label = function() { var o = PeptideRuntime.object; return o == null || o.__mc == null ? null : o.__mc.currentLabel; };
	// a stat of the attack it is in, for another object's code
	self.exports.__attackStat = function(k) { return Attacks.stat(PeptideRuntime.object, k); };
	// a field of one of its clips ("self" its own object, "root" its clip, "stance" the clip its
	// animation shows), as a container another object's code reads and writes
	self.exports.__fieldOf = function(clip, name) {
		var o = PeptideRuntime.object;
		var t = clip == "self" ? o : (clip == "root" ? o.__mc : PeptideRuntime.stanceOf(o.__mc));
		if (t == null) return null;
		return {
			get: function() { var c = AS3.at(t, "__c_" + name); return c != null ? c.get() : AS3.at(t, name); },
			set: function(v) { var c = AS3.at(t, "__c_" + name); if (c != null) c.set(v); else AS3.putKey(t, name, v); }
		};
	};
	self.exports.__traceStart = function() { Rollback.trace = []; Rollback.engineOf = []; Rollback.replayed = 0; Rollback.mismatches = []; Rollback.traceFrom = Rollback.tick.get(); return Rollback.traceFrom; };
	self.exports.__traceLength = function() { return Rollback.trace == null ? 0 : Rollback.trace.length; };
	self.exports.__traceRead = Rollback.traceRead;
	self.exports.__traceReplays = function() { return Rollback.replayed + " replayed, " + Rollback.mismatches.length + " differ" + (Rollback.mismatches.length > 0 ? ": " + Rollback.mismatches.slice(0, 3).join(" ; ") : ""); };
}
function update() { PeptideRuntime.update(); }
function onTeardown() { PeptideRuntime.teardown(self); }

// what the verify gates ask the script (tools/verify.py): its frame count, and every animation
// the runtime can play, checked against the entity it runs on
function __verifyPulse() { return PeptideRuntime.frame; }
function __selfCheck() {
	var bad = [];
	var seen = 0;
	for (p in PeptideRuntime.data.pieces) {
		seen++;
		if (!self.hasAnimation(p.anim)) bad.push(p.anim);
	}
	return bad.length == 0 ? "ok " + seen : bad.length + " " + bad.slice(0, 4).join(",");
}
