// Thwomp deck — the SSF2 createSelfPlatform box riding the thwomp (standable in
// fall/land/rise; parked off-world while the thwomp waits at its spawn point).
var COLS_X = [293.6, 445.7, 597.8, 950.1, 1102.2, 1254.3];
var OFF_X = 6.5;
var OFF_Y = -156.0;
var ENGAGE_Y = 72.9;
var m_engaged = self.makeBool(false);

function findThwomp() {
	var objs = match.getCustomGameObjects();
	for (i in 0...objs.length) {
		var o = objs[i];
		for (j in 0...COLS_X.length) { if (Math.abs(o.getX() - COLS_X[j]) < 2) { return o; } }
	}
	return null;
}

// the engine carries a standing rider through ANY structure move (even a far teleport), so
// the dismount blacklists every character FIRST (they detach in place and fall, like the
// SSF2 thwomp despawning under them), then parks; engaging lifts the blacklist again.
function setRiders(allowed:Bool) {
	var chars = match.getCharacters();
	for (i in 0...chars.length) {
		if (allowed) { self.removeFromBlacklist(chars[i]); } else { self.addToBlacklist(chars[i]); }
	}
}

function update() {
	var t = findThwomp();
	if (t != null && t.getY() > ENGAGE_Y) {
		if (!m_engaged.get()) { m_engaged.set(true); setRiders(true); }
		self.setX(t.getX() + OFF_X); self.setY(t.getY() + OFF_Y);
	} else {
		if (m_engaged.get()) { m_engaged.set(false); setRiders(false); }
		self.setX(-2000); self.setY(-3000);
	}
}
function initialize() {}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}
