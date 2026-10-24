// The entry the engine calls on a stage's script. Emitted last, after the stage's data.

// what the running stage says it made and how far it has run, in the script's own scope, where the
// verify gates ask (tools/verify.py): its frame count, whether its boot finished, and each thing it
// made as `id|kind|x|y|frame|total;` (`|null` or `|gone` for one that is not there; a structure
// has no frames; `|held` after a picture its code holds on a frame)
var __made = [];
var __verifyFrames = 0;
var __verifyBuilt = false;
function __verifyDump() {
	var s = "";
	for (m in __made) {
		var o = m.o;
		var line = m.id + "|" + m.k;
		if (o == null) line = line + "|null";
		else if (o.isDisposed()) line = line + "|gone";
		else {
			line = line + "|" + o.getX() + "|" + o.getY();
			if (m.k != "s") line = line + "|" + o.getCurrentFrame() + "|" + o.getTotalFrames();
			if (m.k == "v" && Planes.holds(o)) line = line + "|held";
		}
		s = s + line + ";";
	}
	return s;
}
self.exports.__verifyDump = __verifyDump;

// the bodies the tree placed are part of the stage from its first frame, ahead of the fighters
// (made as the stage starts: `Stages.begin`)
function initialize() {
	PeptideRuntime.start(Stages.engine(self), DATA, Stages);
}
function update() {
	__verifyFrames = __verifyFrames + 1;
	PeptideRuntime.update();
}
function onTeardown() { Stages.teardown(); }
