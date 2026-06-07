// Stats for battlefieldssf2 (converted from SSF2)

{
	spriteContent: self.getResource().getContent("battlefieldssf2"),
	animationId: "stage",
	ambientColor: 0xffffffff,
	shadowLayers: [],
	camera: {
		startX: 0,
		startY: 0,
		zoomX: 0,
		zoomY: 0,
		camEaseRate: 1 / 11,
		camZoomRate: 1 / 15,
		minZoomHeight: 360,
		initialHeight: 360,
		initialWidth: 640,
		backgrounds: [
			{
				spriteContent: self.getResource().getContent("battlefieldssf2"),
				animationId: "parallax0",
				mode: ParallaxMode.BOUNDS,
				originalBGWidth: 882,
				originalBGHeight: 611,
				horizontalScroll: false,
				verticalScroll: false,
				loopWidth: 0,
				loopHeight: 0,
				xPanMultiplier: 0.4,
				yPanMultiplier: 0.4,
				scaleMultiplier: 1,
				foreground: false,
				depth: 2000
			}
		]
	}
}
