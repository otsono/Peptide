// The entry the engine calls on a stage object's script. Emitted last, after the object's data.

function initialize() {
	Bodies.start(self, DATA);
	self.exports.__verifyPulse = __verifyPulse;
}
function update() { PeptideRuntime.update(); }
// its body stops standing as it is killed, and its script ends once the engine lets it go
function onKill() { if (PeptideRuntime.object != null) Bodies.end(PeptideRuntime.object); }
function onTeardown() { Bodies.teardown(self); }

// what the verify gates ask the script (tools/verify.py): its frame count
function __verifyPulse() { return PeptideRuntime.frame; }
