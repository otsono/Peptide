// Sinking platform — 1:1 from the SSF2 BowsersCastlePlatform disasm, frame-doubled.
var SINK_SPEED = 19.5;
var RISE_SPEED = 0.65;
var SINK_DEPTH = 188.5;
var WAIT = 780;
var HALF_W = 224.0;
var m_init = self.makeBool(false);
var m_startY = self.makeFloat(0.0);
var m_startX = self.makeFloat(0.0);
var m_action = self.makeInt(0);
var m_timer = self.makeInt(0);
var m_shake = self.makeBool(false);

function thwompLanded() {
	var objs = match.getCustomGameObjects();
	var px = m_startX.get();
	for (i in 0...objs.length) {
		var o = objs[i];
		if (Math.abs(o.getX() - px) < HALF_W && Math.abs(o.getY() - m_startY.get()) < 90) { return true; }
	}
	return false;
}

function update() {
	if (!m_init.get()) { m_init.set(true); m_startY.set(self.getY()); m_startX.set(self.getX()); }
	var a = m_action.get();
	if (a == 0) { if (thwompLanded()) { m_action.set(1); } }
	else if (a == 1) { // sink + shake
		var sh = -1.3; if (m_shake.get()) { sh = 1.3; }
		self.setX(m_startX.get() + sh); m_shake.set(!m_shake.get());
		self.setY(self.getY() + SINK_SPEED);
		if (self.getY() >= m_startY.get() + SINK_DEPTH) { self.setY(m_startY.get() + SINK_DEPTH); self.setX(m_startX.get()); m_action.set(2); m_timer.set(0); }
	} else if (a == 2) { m_timer.inc(); if (m_timer.get() >= WAIT) { m_action.set(3); } }
	else { // rise
		self.setY(self.getY() - RISE_SPEED);
		if (self.getY() <= m_startY.get()) { self.setY(m_startY.get()); m_action.set(0); }
	}
}
function initialize() {}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}
