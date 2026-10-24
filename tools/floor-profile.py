#!/usr/bin/env python3
"""floor profile: drop a fighter at each x on both engines and print where it comes to rest,
in the source's units, so a missing roof, a wall that ejects on one side only, or a platform
a few px off shows up as a row. paces on frames (speed 8 + tick), never on the clock.

usage: tools/floor-profile.py <stage> [--from X] [--to X] [--step N] [--drop Y] [--frames N]
"""
import argparse, json, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verify as V


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stage")
    ap.add_argument("--from", dest="x0", type=float, default=-300)
    ap.add_argument("--to", dest="x1", type=float, default=1200)
    ap.add_argument("--step", type=float, default=50)
    ap.add_argument("--drop", type=float, default=-200, help="y to drop from, source units")
    ap.add_argument("--frames", type=int, default=60)
    ap.add_argument("--tol", type=float, default=2.0)
    a = ap.parse_args()
    cid = a.stage if a.stage.endswith("ssf2") else a.stage + "ssf2"
    plan_path = os.path.join(V.ROOT, "stages", cid, "verify.json")
    if not os.path.exists(plan_path):
        # the gates clean their output up; convert into the build tree for the plan (the
        # package the engine boots is the one already published)
        out = os.path.join(V.ROOT, "build", "profile")
        subprocess.run([V.BIN, "ssf2", "stage", V.source_ssf(a.stage, "stage"), "--out", out, "--yes"],
                       capture_output=True)
        plan_path = os.path.join(out, cid, "verify.json")
    plan = json.load(open(plan_path, encoding="utf8"))
    (ox, oy), sc = plan["fighter_origin"], plan["scale"]
    xs = []
    x = a.x0
    while x <= a.x1 + 1e-9:
        xs.append(x); x += a.step
    subprocess.run("pkill -x hl; pkill -f 'peptide session'", shell=True, capture_output=True)
    time.sleep(0.5)
    s = V.Ssf2Engine("profile-s")
    if s.boot(["--char", "mario,mario", "--stage", a.stage]) is None:
        sys.exit("ssf2 did not boot")
    s.tell("speed 8")
    src = {}
    for x in xs:
        s.tell(f"e p0.X = {x}"); s.tell(f"e p0.Y = {a.drop}"); s.tell(f"tick {a.frames}")
        src[x] = (float(V.ssf2_read(s, "e p0.X")), float(V.ssf2_read(s, "e p0.Y")))
    subprocess.run("pkill -x hl; pkill -f 'peptide session'", shell=True, capture_output=True)
    time.sleep(0.5)
    e = V.Engine("profile-f")
    if e.boot(["--stage", cid], ("engine READY", "LAUNCHED")) is None:
        sys.exit("fraymakers did not boot")
    e.tell("speed 8")
    rd = lambda q: e.reply(e.tell("e " + q), "E:", 5)
    port = {}
    for x in xs:
        fx, fy = (x - ox) * sc, (a.drop - oy) * sc
        e.reply(e.tell(f"e match.getCharacters()[0].setX({fx:.2f}); match.getCharacters()[0].setY({fy:.2f}); "
                       f"match.getCharacters()[0].setYVelocity(1); match.getCharacters()[0].toState(CState.FALL)"), "E:", 5)
        e.tell(f"tick {a.frames}")
        gx, gy = float(rd("match.getCharacters()[0].getX()")), float(rd("match.getCharacters()[0].getY()"))
        port[x] = (gx / sc + ox, gy / sc + oy)
    subprocess.run("pkill -x hl; pkill -f 'peptide session'", shell=True, capture_output=True)
    bad = 0
    print(f"{'drop x':>8} | {'ssf2 x':>8} {'ssf2 y':>8} | {'fm x':>8} {'fm y':>8} | note")
    for x in xs:
        (sx, sy), (px, py) = src[x], port[x]
        note = ""
        if abs(sy - py) > a.tol:
            note = f"REST DIFFERS by {py - sy:+.1f}"; bad += 1
        elif abs(sx - px) > a.tol:
            note = f"pushed differently ({px - sx:+.1f})"; bad += 1
        print(f"{x:8.0f} | {sx:8.1f} {sy:8.1f} | {px:8.1f} {py:8.1f} | {note}")
    print(f"{len(xs) - bad}/{len(xs)} columns rest alike (tol {a.tol} px)")


if __name__ == "__main__":
    main()
