// Sinking platform (converted from SSF2 BowsersCastlePlatform). The structure moves itself;
// a falling Thwomp landing on it triggers the sink (SSF2's thwomp.sink() call).
var SINK_SPEED = 3.0;
var RISE_SPEED = 2.0;
var SINK_DEPTH = 210.0;
var WAIT_FRAMES = 120;
var HALF_W = 150.0;
var m_startY = self.makeFloat(0.0);
var m_action = self.makeInt(0);
var m_timer = self.makeInt(0);

function initialize() {
	m_startY.set(self.getY());
}

function thwompLanded() {
	// a Thwomp (the only custom game object near my idle surface) is on top of me.
	var objs = match.getCustomGameObjects();
	var px = self.getX();
	for (i in 0...objs.length) {
		var o = objs[i];
		if (Math.abs(o.getX() - px) < HALF_W && Math.abs(o.getY() - m_startY.get()) < 70) { return true; }
	}
	return false;
}

function update() {
	var a = m_action.get();
	if (a == 0) {
		if (thwompLanded()) { m_action.set(1); }
	} else if (a == 1) {
		self.setY(self.getY() + SINK_SPEED);
		if (self.getY() >= m_startY.get() + SINK_DEPTH) { self.setY(m_startY.get() + SINK_DEPTH); m_action.set(2); m_timer.set(0); }
	} else if (a == 2) {
		m_timer.set(m_timer.get() + 1);
		if (m_timer.get() >= WAIT_FRAMES) { m_action.set(3); }
	} else {
		self.setY(self.getY() - RISE_SPEED);
		if (self.getY() <= m_startY.get()) { self.setY(m_startY.get()); m_action.set(0); m_timer.set(0); }
	}
}

function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}
