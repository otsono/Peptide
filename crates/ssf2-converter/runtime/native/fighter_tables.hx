// The tables a fighter written as Fraymakers code reads, worked out from what it declares
// (emit/native_fighter.rs): each block's `// {cases}` line is replaced by the cases its package
// gives it. Edit them here.

// ---- attack_label
// the attack (the source's label) each animation shows
function attackLabel(anim) {
	switch (anim) {
		// {cases}
	}
	return null;
}

// ---- declared
// what each attack declares, of the stats its rules and its code read
function declared(label, k) {
	switch (label) {
		// {cases}
	}
	return null;
}

// ---- declared_box
// what each attack's boxes declare, of the stats its rules read, by the source's box number
function declaredBox(label, i, k) {
	switch (label) {
		// {cases}
	}
	return null;
}

// ---- label_anims
// the animations each of the source's labels its code turns off shows
function labelAnims(label) {
	switch (label) {
		// {cases}
	}
	return [];
}

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
// where its item box is, in the source's units from its origin, on the frame it shows: from the
// frame each row starts on, [frame, x, y, ...]
function itemBox() {
	var k = Std.int((self.getCurrentFrame() - 1) / 2);
	switch (self.getAnimation()) {
		// {cases}
	}
	return { x: 0, y: 0 };
}
function boxAt(k, row) {
	var i = 0;
	var at = 0;
	while (i < row.length) { if (row[i] <= k) at = i; i += 3; }
	return { x: row[at + 1], y: row[at + 2] };
}

// ---- item_content
// the content each item it can make is
function itemContent(name) {
	switch (name) {
		// {cases}
	}
	return null;
}

// ---- item_declared
// what each item it can make declares
function itemDeclared(name, k) {
	switch (name) {
		// {cases}
	}
	return null;
}

// ---- fired_declared
// what each projectile it fires declares, of the stats its code reads
function firedDeclared(name, k) {
	switch (name) {
		// {cases}
	}
	return null;
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
	var c = clip == "self" ? ITSELF : (clip == "root" ? ROOT : stanceClass());
	switch (c) {
		// {cases}
	}
	return null;
}

// ---- stance_class
// the class of the clip each animation shows
function stanceClass() {
	switch (self.getAnimation()) {
		// {cases}
	}
	return null;
}
