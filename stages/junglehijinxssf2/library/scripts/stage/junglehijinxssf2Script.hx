// API Script for junglehijinxssf2 (converted from SSF2)

var m_hazardsSpawned = false;
function initialize() {
	self.pause();
}
function update() {
	if (!m_hazardsSpawned) {
		var chars = match.getCharacters();
		if (chars.length > 0) {
			m_hazardsSpawned = true;
			var owner = null;
			var _bg0 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("junglehijinxssf2_hijinx_highestbirds_bg"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg0 != null) {
				self.getBackgroundBehindContainer().addChild(_bg0.getSprite());
			}
			var _bg1 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("junglehijinxssf2_hijinx_birdslow_bg"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg1 != null) {
				self.getBackgroundBehindContainer().addChild(_bg1.getSprite());
			}
			var _bg2 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("junglehijinxssf2_hijinx_highbirds_bg"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg2 != null) {
				self.getBackgroundBehindContainer().addChild(_bg2.getSprite());
			}
			var _bg3 = match.createVfx(new VfxStats({ spriteContent: self.getResource().getContent("junglehijinxssf2_hijinx_lighting"), animation: "active", x: 0, y: 0, loop: true, timeout: -1, relativeWith: false, resizeWith: false }));
			if (_bg3 != null) {
				self.getBackgroundBehindContainer().addChild(_bg3.getSprite());
			}
		}
	}
}
function onTeardown() {}
function onKill() {}
function onStale() {}
function afterPushState() {}
function afterPopState() {}
function afterFlushStates() {}