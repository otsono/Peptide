// The floor as the source tells a projectile of it (compile/native.rs takes the blocks a
// projectile's code needs, under their `// ---- name` lines).

// ---- told
// the source tells a projectile it is on the floor the first frames it finds itself there (one
// made on it too: pikachu's jolt is made at his feet and starts its hops from that touch), where
// this engine tells only one that comes down onto it
var toldFloor = self.makeObject(false);

// whether a floor is right under its feet (made this frame, the engine has not put it on one yet)
function floorBelow() {
	var a = Point.create(self.getX(), self.getY() - 2);
	var b = Point.create(self.getX(), self.getY() + 2);
	var hits = self.hitTestStructuresWithLineSegment(a, b, Point.create(b.x, b.y), { structureType: StructureType.FLOOR });
	return hits != null && hits.length > 0;
}

// its floor listeners told of the floor its first frames find it on, once its age has counted the frame
function floorFrame() {
	if (!toldFloor.get() && age.get() <= 2 && (self.isOnFloor() || (age.get() == 1 && floorBelow()))) {
		toldFloor.set(true);
		// {cases}
	}
}

// ---- speed
// the source tells a floor listener of the touch with the speed it came down at, which this
// engine's landing has already taken off (link's boomerang bounces back up by it): one made on
// the floor, with the speed it was launched at, which its maker hands over
var airX = self.makeFloat(0);
var airY = self.makeFloat(0);
// a rise its code gave it as it touched, kept past the engine's own landing
var risen = self.makeObject(false);
var riseX = self.makeFloat(0);
var riseY = self.makeFloat(0);

// a floor listener's code, run with the speed it came down at; the engine's own speed back after
// unless that code changed it
function onFloor(listener) {
	var across = self.getXSpeed();
	var given = self.getYSpeed() == 0 && airY.get() != 0;
	if (given) {
		self.setXSpeed(airX.get());
		self.setYSpeed(airY.get());
	}
	listener(null);
	if (self.isDisposed()) return;
	if (given && self.getXSpeed() == airX.get() && self.getYSpeed() == airY.get()) {
		self.setXSpeed(across);
		self.setYSpeed(0);
	} else if (self.getYSpeed() < 0) {
		risen.set(true);
		riseX.set(self.getXSpeed());
		riseY.set(self.getYSpeed());
	}
}

// each frame, once its age has counted it: a rise kept, and the speed it has in the air kept for
// the next touch (its launch, on its first)
function airFrame() {
	if (risen.get()) {
		risen.set(false);
		self.setXSpeed(riseX.get());
		self.setYSpeed(riseY.get());
	}
	var launched = self.exports.__launched;
	if (age.get() == 1 && launched != null && launched.xs != null) {
		airX.set(launched.xs);
		airY.set(launched.ys);
	} else if (!self.isOnFloor() || age.get() == 1) {
		airX.set(self.getXSpeed());
		airY.set(self.getYSpeed());
	}
}
