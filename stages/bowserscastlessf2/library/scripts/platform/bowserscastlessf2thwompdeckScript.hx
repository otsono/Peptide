// Thwomp deck — the SSF2 createSelfPlatform box riding the thwomp (standable in
// fall/land/rise; parked off-world while the thwomp waits at its spawn point).
var COLS_X = [293.6, 445.7, 597.8, 950.1, 1102.2, 1254.3];
var OFF_X = 6.5;
var OFF_Y = -156.0;
var ENGAGE_Y = 72.9;

function findThwomp() {
	var objs = match.getCustomGameObjects();
	for (i in 0...objs.length) {
		var o = objs[i];
		for (j in 0...COLS_X.length) { if (Math.abs(o.getX() - COLS_X[j]) < 2) {
			return o;
		} }
	}
	return null;
}

function update() {
	var t = findThwomp();
	if (t != null && t.getY() > ENGAGE_Y) {
		self.setX(t.getX() + OFF_X);
		self.setY(t.getY() + OFF_Y);
	}
	// parking carries any remaining rider off-world -> KO, the SSF2 outcome for riding
	// the thwomp to the top (it crosses the top blast bound there).
	else {
		self.setX(-2000);
		self.setY(-3000);
	}
}
function initialize() {}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}