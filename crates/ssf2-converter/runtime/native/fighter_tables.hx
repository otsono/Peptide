// The tables a fighter written as Fraymakers code reads, worked out from what it declares
// (emit/native_fighter.rs): each block's `// {cases}` line is replaced by the cases its package
// gives it. Edit them here.

// ---- clip_label
// the label its clip is on, as the source reads it (the last at or before the frame it shows)
function clipLabel() {
	var k = Std.int((self.getCurrentFrame() - 1) / 2);
	switch (self.getAnimation()) {
		// {cases}
	}
	return null;
}

// ---- item_box
//: SIZE the source's units in this engine's
// where its item box is, in the source's units from its origin, on the frame it shows: the custom
// box each animation that places one carries (`CUSTOM_BOX_A`, written from the source's); on a
// frame that places none, where it last was
function itemBox() {
	var bs = self.getCollisionBoxes(CollisionBoxType.CUSTOMA);
	if (bs == null || bs.length == 0) return self.exports.__ibx != null ? { x: self.exports.__ibx, y: self.exports.__iby } : { x: 0, y: 0 };
	return { x: bs[0].centerX / SIZE, y: bs[0].centerY / SIZE };
}

// ---- item_content
//: NAMES the items it can make, by name
// the content each item it can make is
function itemContent(name) {
	switch (name) {
		// {cases}
	}
	return null;
}
// any one of the items it can make (the source's "random")
function anyItemName() {
	var all = [NAMES];
	return all.length == 0 ? null : all[Random.getInt(0, all.length - 1)];
}

// ---- anim_of
// the animation each of the source's labels is
function animOf(label) {
	switch (label) {
		// {cases}
	}
	return null;
}

// ---- field_of
//: ITSELF its own class, ROOT its clip's class
// the container a field of one of its clips is kept in, for another object's code ("self" its own
// class, "root" its clip, "stance" the clip its animation shows)
function fieldOf(clip, name) {
	var c = clip == "self" ? ITSELF : (clip == "root" ? ROOT : self.getAnimationStatsMetadata().clip);
	switch (c) {
		// {cases}
	}
	return null;
}

// ---- clip_label_meta
// the label its clip is on, as the source reads it (the last at or before the frame it shows): its
// animation's metadata says, `clipLabel` as it starts and `clipLabels` [frame, label] inside it
function clipLabel() {
	var m = self.getAnimationStatsMetadata();
	if (m == null) return null;
	var at = m.clipLabel;
	if (m.clipLabels == null) return at;
	var k = Std.int((self.getCurrentFrame() - 1) / 2);
	for (l in m.clipLabels) if (k >= l[0]) at = l[1];
	return at;
}

// ---- root_label_meta
// the source's label its animation shows (its metadata's `label`), for another object's code
function rootLabel() {
	var m = self.getAnimationStatsMetadata();
	return m == null ? null : m.label;
}

