// ---- forms
// a fighter whose package's code turns it into another fighter of the package (`replaceCharacter`):
// the other one merged into it as its second form, its names under PREFIX. which form it is (0 its
// own, 1 the other), and whether the second has started
var form = self.makeInt(0);
var formStarted = self.makeBool(false);
// each form's listeners and timers run only while it is that form
var formListeners = [];
function formListen(k, t, h, ?o) {
	for (l in formListeners) if (l.k == k && l.t == t && l.h == h) return;
	var w = function(e) { if (form.get() == k) h(e); };
	formListeners.push({ k: k, t: t, h: h, w: w });
	self.addEventListener(t, w, o);
}
function formUnlisten(k, t, h) {
	for (l in formListeners.copy()) if (l.k == k && l.t == t && l.h == h) { self.removeEventListener(t, l.w); formListeners.remove(l); }
}
function formTimer(k, n, r, f, ?o) {
	return self.addTimer(n, r, function() { if (form.get() == k) f(); }, o);
}
// the second form's code names its animations as its source does; its own are under the prefix
function formAnim(n) { return (form.get() == 1 && n != null && self.hasAnimation("PREFIX" + n)) ? "PREFIX" + n : n; }
function formPlay(n) { self.playAnimation(formAnim(n)); }
function formHas(n) { return self.hasAnimation(formAnim(n)); }
function formBase(n) { return (n != null && n.indexOf("PREFIX") == 0) ? n.substr(PREFIX_LEN) : n; }
// the engine plays its states' animations by their names: the form's own, while it is that form
function formFrame() {
	var a = self.getAnimation();
	if (a == null) return;
	var want = form.get() == 1 ? formAnim(formBase(a)) : formBase(a);
	if (want != a && self.hasAnimation(want)) self.playAnimation(want);
}
// what each form publishes for other objects' code, put back as it takes over
function formPublish(k) {
	if (k == 0) {
		// {publish_a}
	} else {
		// {publish_b}
	}
}
// turning into the other form (`replaceCharacter(id, attack, label)`), going on in its attack at
// the label: its stats, its first start, what it publishes, and the attack by its source label
function formTurn(id, attack, label) {
	var k = id == "B_ID" ? 1 : (id == "A_ID" ? 0 : -1);
	if (k < 0 || k == form.get()) return;
	form.set(k);
	self.updateCharacterStats(k == 1 ? STATS_B : STATS_A);
	if (k == 1 && !formStarted.get()) { formStarted.set(true); PREFIXinitialize(); }
	formPublish(k);
	self.exports.__formSwitch = formTurn;
	var want = formAttack(k, attack);
	if (want != null) { self.playAnimation(want); if (label != null) self.playFrameLabel(label); }
}
// each form's attacks by their source labels, as its animations' metadata names them
function formAttack(k, attack) {
	switch (k + "/" + attack) {
		// {cases}
	}
	return null;
}
