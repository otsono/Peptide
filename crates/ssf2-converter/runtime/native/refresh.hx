// A projectile's attack hits again every `refreshRate` of the source's frames (ness's fire pillar
// burns a fighter standing in it every eight): on its Nth source frame and every N after, a timer
// that stands still while it is frozen. compile/native.rs takes this block where it declares a
// rate or its code writes one.

// ---- projectile
var refreshTimer = self.makeInt(-1);
var refreshEvery = self.makeInt(0);
function setRefresh(r) {
	// (none, -1, or 999 and up: a rate the source never refreshes on)
	var every = r != null && r > 0 && r < 999 ? Std.int(r * 2) : 0;
	if (every == refreshEvery.get()) return;
	refreshEvery.set(every);
	if (refreshTimer.get() >= 0) self.removeTimer(refreshTimer.get());
	refreshTimer.set(-1);
	if (every == 0) return;
	refreshTimer.set(self.addTimer(every - 1, 1, refreshFirst, { persistent: true, pauseCondition: refreshPaused }));
}
function refreshPaused() { return self.getHitstop() > 0; }
function refreshFirst() {
	if (self.isDisposed()) return;
	self.reactivateHitboxes();
	refreshTimer.set(self.addTimer(refreshEvery.get(), -1, refreshAgain, { persistent: true, pauseCondition: refreshPaused }));
}
function refreshAgain() {
	if (!self.isDisposed()) self.reactivateHitboxes();
}
