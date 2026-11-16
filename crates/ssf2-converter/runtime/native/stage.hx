// {title}: converted from SSF2 by Peptide.
//
// written as Fraymakers code: what the stage puts down as the match starts (its boot, worked out at
// conversion), what it keeps up each frame, and what its own code ({classes}) does, as this
// engine's calls. the parts every stage has come from runtime/native/stage.hx and
// runtime/native/stage_boot.hx; edit those files, not this one.

// ---- what the running stage says it made and how far it has run, where the verify gates ask
// (tools/verify.py): its frame count, whether its boot finished, and each thing it made as
// `id|kind|x|y|frame|total;` (`|null` or `|gone` for one that is not there; a structure has no frames;
// `|held` after a picture its code holds on a frame)
var __made = [];
var __verifyFrames = 0;
var __verifyBuilt = false;
// the pictures its code holds on a frame (`|held` after one in the dump)
var heldPieces = [];
function pieceHeld(v, on) {
	if (v == null) return;
	heldPieces.remove(v);
	if (on) { heldPieces.push(v); if (v.pause != null) v.pause(); } else if (v.resume != null) v.resume();
}
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
			if (m.k == "v" && heldPieces.indexOf(o) >= 0) line = line + "|held";
		}
		s = s + line + ";";
	}
	return s;
}
self.exports.__verifyDump = __verifyDump;

// ---- what the boot puts down and the stage's frame keeps: what it owns (gone with the stage),
// containers its timeline shows and hides, colour and opacity schedules, pieces riding an object
var m_owned = [];
var m_shown = [];
var m_tint = [];
var m_alpha = [];
var __follow = [];
// how many of its frames it has run since the boot
var stageFrames = 0;
// {fields}

// ---- its own code, and the pieces its boot puts down
// {functions}

// ---- what every stage does
// the bodies the tree placed are part of the stage from its first frame, ahead of the fighters
function initialize() {
	// {placed}
	// {publish}
}

// what the stage puts down once there are fighters (the match's hazards switch decides what the
// source put behind it); then its own code starts
function bootStage() {
	var chars = match.getCharacters();
	if (chars == null || chars.length == 0) return;
	var cfg = match.getMatchSettingsConfig();
	var hazardsOn = cfg == null || cfg.hazards;
	// {spawn}
	__verifyBuilt = true;
	// {own_start}
}

function update() {
	__verifyFrames = __verifyFrames + 1;
	if (!__verifyBuilt) {
		bootStage();
		if (!__verifyBuilt) return;
	}
	stageFrames = stageFrames + 1;
	var t = self.getCurrentFrame() > 1 ? self.getCurrentFrame() : stageFrames;
	// {upkeep}
	// what its own class runs each of the source's frames (every second engine frame)
	if (stageFrames % 2 == 1) {
		// {own_tick}
	}
}

// the stage ends: what it made for itself goes with it
function onTeardown() {
	for (o in m_owned) if (o != null) o.destroy();
	m_owned = [];
}
