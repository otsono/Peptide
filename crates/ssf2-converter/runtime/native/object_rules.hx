// What every stage object is played by, as Fraymakers code: its body (the floor and walls it stands
// for), the floor its own code gives it, its hits coming round again. A stage object written natively
// carries only the blocks it uses (`emit/stage.rs`).

// ---- faces
//: the faces of a body, made where nobody can reach them and moved onto the object each frame. the
//: engine's structures cannot be taken away, so a face an object has finished with waits on the
//: stage for the next one of the same shape
function spareFaces() {
	var ex = stage == null ? null : stage.exports;
	if (ex == null) return [];
	if (ex.__spareFaces == null) ex.__spareFaces = [];
	return ex.__spareFaces;
}
function makeFace(ty, pts, ledgeL, ledgeR, drop) {
	var key = ty + ":" + pts.join(",") + ":" + ledgeL + ledgeR + drop;
	var pool = spareFaces();
	var k = 0;
	var st = null;
	while (k < pool.length) {
		if (pool[k].k == key) { st = pool[k].st; pool.splice(k, 1); break; }
		k++;
	}
	if (st == null) {
		st = match.createLineSegmentStructure(pts, new StructureStats({ startX: -2000, startY: -3000, structureType: ty, leftLedge: ledgeL, rightLedge: ledgeR, dropThrough: drop }));
		if (st == null) return null;
	}
	st.updateStructureStats({ disabled: false });
	// (whose face it is, for another object's code that asks what a fighter stands on)
	var ex = stage == null ? null : stage.exports;
	if (ex != null) {
		if (ex.__faceOwners == null) ex.__faceOwners = [];
		var known = false;
		for (fo in ex.__faceOwners) if (fo.st == st) { fo.o = self; known = true; }
		if (!known) ex.__faceOwners.push({ st: st, o: self });
	}
	// a body never stands on its own faces
	st.addToBlacklist(self);
	st.setX(self.getX());
	st.setY(self.getY());
	return { st: st, k: key, x: null, y: null, drop: drop == true };
}
// an object let through its faces, or no longer (what it publishes as `__ignore`)
function ignoreThrough(o, on) {
	for (f in bodyFacesOf()) { if (on) f.st.addToBlacklist(o); else f.st.removeFromBlacklist(o); }
}
// every face it has now: its body's and its own floor's (each block keeps its own list)
function bodyFacesOf() {
	var out = [];
	if (BODY_ON) for (f in bodyFaces) out.push(f);
	if (FLOOR_ON) for (f in ownFaces) out.push(f);
	return out;
}
// whether fighters drop through its faces, set by another object's code (what it publishes as
// `__through`)
self.exports.__through = function(on) { for (f in bodyFacesOf()) f.st.updateStructureStats({ dropThrough: on == true }); };
function releaseFaces(faces) {
	var pool = spareFaces();
	for (f in faces) {
		f.st.updateStructureStats({ disabled: true });
		f.st.setX(-2000);
		f.st.setY(-3000);
		f.st.removeFromBlacklist(self);
		pool.push(f);
	}
}
// each face goes where the object is (written only when it is not already there)
function facesFollow(faces) {
	var x = self.getX();
	var y = self.getY();
	for (f in faces) if (f.x != x || f.y != y) { f.st.setX(x); f.st.setY(y); f.x = x; f.y = y; }
}

// ---- placed_body
//: the body the tree placed it as (BODY: the outline its collision clip draws, else the box it
//: declares, this engine's units): part of the stage, which the death bounds never take
var bodyFaces = [];
function placedBody() {
	var b = BODY;
	var runs = b.faces;
	if (runs == null || runs.length == 0) {
		var l = b.box[0]; var t = b.box[1]; var r = b.box[2]; var bt = b.box[3];
		runs = [[StructureType.FLOOR, [l, t, r, t]], [StructureType.RIGHT_WALL, [r, t, r, bt]], [StructureType.CEILING, [r, bt, l, bt]], [StructureType.LEFT_WALL, [l, bt, l, t]]];
	}
	for (run in runs) {
		var floor = run[0] == StructureType.FLOOR;
		var f = makeFace(run[0], run[1], floor && b.ledges[0] == true, floor && b.ledges[1] == true, false);
		if (f != null) bodyFaces.push(f);
	}
	self.updateGameObjectStats({ deathBoundsDestroy: false });
}

// ---- own_floor
//: the body an object gives itself (source units, its own space): a floor fighters land on from
//: above and drop through, or, asked for as terrain, a solid box with ledges. one at a time
var ownFaces = [];
function ownFloor(x, y, w, h, ?terrain) {
	dropOwnFloor();
	var x0 = Math.min(x, x + w) * SIZE;
	var x1 = Math.max(x, x + w) * SIZE;
	var y0 = Math.min(y, y + h) * SIZE;
	var y1 = Math.max(y, y + h) * SIZE;
	var solid = terrain == true;
	var runs = solid
		? [[StructureType.FLOOR, [x0, y0, x1, y0]], [StructureType.RIGHT_WALL, [x1, y0, x1, y1]], [StructureType.CEILING, [x1, y1, x0, y1]], [StructureType.LEFT_WALL, [x0, y1, x0, y0]]]
		: [[StructureType.FLOOR, [x0, y0, x1, y0]]];
	for (run in runs) {
		var floor = run[0] == StructureType.FLOOR;
		var f = makeFace(run[0], run[1], floor && solid, floor && solid, floor && !solid);
		if (f != null) ownFaces.push(f);
	}
	return ownFaces.length == 0 ? null : ownFaces[0].st;
}
function dropOwnFloor() {
	if (ownFaces.length == 0) return;
	var faces = ownFaces;
	ownFaces = [];
	releaseFaces(faces);
}

// ---- rehit
//: the attack it is in hits a fighter again every `refreshRate` of the source's frames (REHIT: each
//: animation's, in engine frames), as long as the fighter stays in it
var rehitAnim = null;
var rehitAt = 0;
var rehitEvery = 0;
function rehitUpkeep() {
	var a = self.getAnimation();
	if (a != rehitAnim) {
		rehitAnim = a;
		rehitAt = 0;
		rehitEvery = 0;
		switch (a) {
			// {cases}
		}
	}
	if (rehitEvery <= 0) return;
	rehitAt++;
	if (rehitAt >= rehitEvery) { rehitAt = 0; self.reactivateHitboxes(); }
}

// ---- faces_through
//: whether fighters drop through its floors (its body's and its own)
var throughNow = self.makeObject(null);
function facesThrough(on) {
	throughNow.set(on == true);
	for (f in bodyFacesOf()) f.st.updateStructureStats({ dropThrough: on == true });
}
// whether they do now: as last set, else as its faces were made
function throughOf() {
	if (throughNow.get() != null) return throughNow.get();
	for (f in bodyFacesOf()) return f.drop == true;
	return false;
}
