// a fighter's parts: the named clips its animations place that its code turns (sends to a frame,
// tints), which the conversion draws apart from the animations' art. each is a vfx of its own
// riding the fighter where the animation it plays places it, hidden where it places none, and
// started over each time an animation that places it starts.
//
// `//:` lines are notes for whoever edits this, left out of what is written.

// ---- place_parts
//: its cases are each animation that places a part, as the parts it places there
// where the animation it plays places each of its parts (offsets in the source's units)
function placeParts() {
	var anim = self.getAnimation();
	// {cases}
}
// one of its parts shown at its place in the animation: the vfx made as it is first shown
// what its code set on a part it reached by its place (`getChildAt`): shown or not, and its
// size, kept until an animation that places it starts again (the source's clip is placed anew)
var partLooks = [];
function partLook(name) {
	for (l in partLooks) if (l.name == name) return l;
	var l = { name: name, visible: true, scaleX: 1.0, scaleY: 1.0, alpha: 1.0 };
	partLooks.push(l);
	return l;
}
function placePart(part, partOn, art, ox, oy, rotation, sx, sy, ?name) {
	var anim = self.getAnimation();
	var v = part.get();
	if (v == null || v.isDisposed()) {
		v = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("CONTENT"), animation: art, loop: true, timeout: -1 }), self);
		part.set(v);
		partOn.set(null);
	}
	if (v == null) return;
	var look = name == null ? null : partLook(name);
	if (partOn.get() != anim) { partOn.set(anim); v.playAnimation(art); v.resume(); if (look != null) { look.visible = true; look.scaleX = 1.0; look.scaleY = 1.0; look.alpha = 1.0; } }
	var facing = self.isFacingRight() ? 1 : -1;
	var turn = self.getRotation() * Math.PI / 180;
	var dx = ox * SIZE;
	var dy = oy * SIZE;
	v.setX(self.getX() + facing * (dx * Math.cos(turn) - dy * Math.sin(turn)));
	v.setY(self.getY() + dx * Math.sin(turn) + dy * Math.cos(turn));
	if (facing < 0) v.faceLeft(); else v.faceRight();
	v.setRotation(self.getRotation() + facing * rotation);
	v.setScaleX(sx * (look == null ? 1 : look.scaleX));
	v.setScaleY(sy * (look == null ? 1 : look.scaleY));
	v.setVisible(look == null || look.visible != false);
	if (look != null) v.setAlpha(look.alpha);
}
// one of its parts sent to a frame by its code: placed first, so an animation that has just
// started (whose first frame's code runs before its update) starts the part over before the jump
function partFrame(part, n, hold) {
	placeParts();
	effectFrame(part.get(), n, hold);
}
// one of its parts as a value (its code sets a colour on it): its vfx, placed first
function partNow(part) {
	placeParts();
	return part.get();
}
// one its animation places none of: hidden, to start over where one does
function hidePart(part, partOn) {
	var v = part.get();
	if (v != null && !v.isDisposed()) v.setVisible(false);
	partOn.set(null);
}
