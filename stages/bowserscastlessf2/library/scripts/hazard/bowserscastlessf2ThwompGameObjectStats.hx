// GameObjectStats for bowserscastlessf2Thwomp
{
	spriteContent: self.getResource().getContent("bowserscastlessf2Thwomp"),
	initialState: PState.ACTIVE,
	stateTransitionMapOverrides: [
		PState.ACTIVE => { animation: "idle" }
	],
	baseScaleX: 1,
	baseScaleY: 1,
	weight: 100,
	gravity: 0,
	friction: 0,
	floorFootPosition: 0,
	floorHeadPosition: 156,
	floorHipWidth: 78,
	aerialFootPosition: 0,
	aerialHeadPosition: 156,
	aerialHipWidth: 78,
}