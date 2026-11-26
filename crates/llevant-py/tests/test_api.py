"""The Python API on the zoo models (tools/mcnp-oracle/models).

Run after `maturin develop --release` in crates/llevant-py: `pytest tests`.
"""

import sys
from pathlib import Path

import numpy as np
import pytest

import llevant

MODELS = Path(__file__).resolve().parents[3] / "tools" / "mcnp-oracle" / "models"
NAMES = ["surfaces", "universes", "periodic"]


@pytest.fixture(scope="module", params=NAMES)
def zoo(request):
    return request.param, llevant.load(MODELS / f"{request.param}.i")


def golden(name):
    rows = [
        line.split()
        for line in (MODELS / f"{name}.golden").read_text().splitlines()
        if not line.startswith("#")
    ]
    pts = np.array([[float(x) for x in r[:3]] for r in rows])
    return pts, np.array([int(r[3]) for r in rows])


def test_classify_agrees_with_mcnp(zoo):
    name, m = zoo
    pts, cells = golden(name)
    r = m.classify(pts)
    assert len(r) == len(pts)
    np.testing.assert_array_equal(r.cell, cells)
    assert (np.asarray(r.status) == llevant.STATUS_NAMES.index("inside")).all()
    for k in range(0, len(pts), 97):
        lab = m.locate(*pts[k])
        assert lab == r[k] and hash(lab) == hash(r[k])
        assert lab.cell == cells[k] == lab.path[-1][0]
    assert r[-1] == r[len(r) - 1]


def test_columns_are_read_only(zoo):
    _, m = zoo
    r = m.classify(np.zeros((3, 3)))
    with pytest.raises(ValueError):
        r.ids[0] = 1


def test_trace_lines_match_trace(zoo):
    _, m = zoo
    lo, hi = m.extent()
    rng = np.random.default_rng(7)
    a = rng.uniform(lo, hi, size=(50, 3))
    b = rng.uniform(lo, hi, size=(50, 3))
    tr = m.trace_lines(a, b)
    assert len(tr) == 50
    off = np.asarray(tr.offsets)
    assert off[0] == 0 and off[-1] == len(tr.t0)
    np.testing.assert_array_equal(np.diff(off), np.bincount(tr.line, minlength=50))
    for i in range(50):
        one = m.trace(a[i], b[i])
        many = tr[i]
        assert len(one) == len(many)
        assert one[0].t0 == 0.0
        assert one[-1].t1 == pytest.approx(np.linalg.norm(b[i] - a[i]))
        for s, t in zip(one, many):
            assert (s.t0, s.t1, s.label) == (t.t0, t.t1, t.label)
            assert s.surface_in == t.surface_in and s.surface_out == t.surface_out
        # Segments cover the line without gaps between them.
        assert all(x.t1 == y.t0 for x, y in zip(one, one[1:]))
    total = sum(tr.lengths("label").values())
    assert total == pytest.approx(np.linalg.norm(b - a, axis=1).sum())
    assert sum(tr.lengths("material").values()) == pytest.approx(total)


def test_trace_with_direction(zoo):
    _, m = zoo
    tr = m.trace_lines([[0, 0, 0], [1, 0, 0]], dir=[[2, 0, 0], [0, 0, 1]], t_max=[10, 5])
    assert tr[0][-1].t1 == 10 and tr[1][-1].t1 == 5
    segs = m.trace((0, 0, 0), dir=(1, 0, 0), t=(0, 10))
    assert [(s.t0, s.t1, s.label) for s in segs] == [(s.t0, s.t1, s.label) for s in tr[0]]


def test_first_hit(zoo):
    _, m = zoo
    lo, hi = m.extent()
    rng = np.random.default_rng(3)
    start = rng.uniform(lo, hi, size=(200, 3))
    d = rng.normal(size=(200, 3))
    h = m.first_hit(start, d)
    assert len(h) == 200 and h.normal.shape == (200, 3)
    hit = np.asarray(h.hit)
    assert hit.any()
    assert np.isinf(np.asarray(h.t)[~hit]).all()
    assert (np.asarray(h.cell)[~hit] == -1).all()
    for k in np.flatnonzero(hit)[:20]:
        assert h[k].status != "on-surface"
    # With every cell hidden, only gaps and problems (periodic.i has an
    # overlap) can be hit.
    hidden = m.first_hit(start, d, hide=list(m.cells), show_void=True)
    assert all(hidden[k].status != "inside" for k in np.flatnonzero(hidden.hit))
    assert (np.asarray(hidden.cell) == -1).all()
    # Hiding materials never hides a filled cell: every hit is in a cell
    # that is not filled, and not of those materials.
    mats = sorted({c.material for c in m.cells.values()} - {0})
    some = m.first_hit(start, d, hide_materials=mats[:1])
    for k in np.flatnonzero(some.hit)[:50]:
        if some[k].status == "inside":
            assert some.material[k] != mats[0]
    with pytest.raises(ValueError, match="no material"):
        m.first_hit(start, d, hide_materials=[987654])


@pytest.mark.parametrize(
    "layout",
    [np.asfortranarray, lambda a: np.repeat(a, 2, axis=0)[::2], lambda a: a.astype(np.float32)],
    ids=["fortran", "strided", "float32"],
)
def test_array_layouts(zoo, layout):
    # Fortran order is what polars and pandas `to_numpy()` give.
    _, m = zoo
    lo, hi = m.extent()
    rng = np.random.default_rng(5)
    # float32 values, so that the float32 layout gives the same points.
    a, b = rng.uniform(lo, hi, size=(2, 40, 3)).astype(np.float32).astype(np.float64)
    d = (b - a).astype(np.float32).astype(np.float64)
    want = m.classify(a)
    got = m.classify(layout(a))
    np.testing.assert_array_equal(got.ids, want.ids)
    t =np.linalg.norm(d, axis=1)
    pairs = [(m.trace_lines(a, b), m.trace_lines(layout(a), layout(b))),
             (m.trace_lines(a, dir=d, t_max=t), m.trace_lines(layout(a), dir=layout(d), t_max=t))]
    for want, got in pairs:
        np.testing.assert_array_equal(got.offsets, want.offsets)
        np.testing.assert_array_equal(got.t1, want.t1)
        np.testing.assert_array_equal(got.ids, want.ids)
    want = m.first_hit(a, d)
    got = m.first_hit(layout(a), layout(d))
    np.testing.assert_array_equal(got.t, want.t)
    np.testing.assert_array_equal(got.ids, want.ids)


def test_slice_image(zoo):
    _, m = zoo
    img = m.slice(z=0.0, width=300)
    assert img.kind == "slice"
    assert img.rgb.shape == (img.height, img.width, 3)
    assert img.cell.shape == img.ids.shape == (img.height, img.width)
    png = img.png()
    assert png[:8] == b"\x89PNG\r\n\x1a\n" and img._repr_png_() == png
    for i, j in [(0, 0), (150, img.height // 2), (299, img.height - 1)]:
        p = img.point(i, j)
        assert p[2] == 0.0
        assert img.at(i, j) == m.locate(p) or img.at(i, j).status == "on-surface"
    with pytest.raises(ValueError):
        img.at(300, 0)


def test_slice_outlines(zoo):
    _, m = zoo
    dark = lambda img: (img.rgb == (40, 40, 40)).all(axis=2)  # the outline colour
    cells = dark(m.slice(z=0.0, width=300))
    for by in ("material", "universe"):
        other = dark(m.slice(z=0.0, width=300, outline_by=by))
        # Each boundary between materials or universes is one between cells.
        assert not (other & ~cells).any(), by
    none = dark(m.slice(z=0.0, width=300, outline=False))
    assert not (none & ~cells).any() and none.sum() < cells.sum()
    with pytest.raises(ValueError):
        m.slice(z=0.0, outline_by="surface")


def test_render_image(zoo, tmp_path):
    _, m = zoo
    img = m.render(size=(80, 60), colour="material")
    assert img.kind == "render" and img.rgb.shape == (60, 80, 3)
    centre = img.at(40, 30)
    if centre is not None:
        assert img.point(40, 30) is not None
    img.save(tmp_path / "r.png")
    assert (tmp_path / "r.png").read_bytes()[:4] == b"\x89PNG"
    lo, hi = m.extent()
    c = tuple((a + b) / 2 for a, b in zip(lo, hi))
    cut = m.render(size=(40, 40), clip=(c, (0, 0, 1)), eye=(c[0], c[1], c[2] + 1e3), target=c)
    assert cut.width == 40


def test_realistic_render(zoo):
    _, m = zoo
    looks = m.materials()
    assert looks and {"material", "look", "colour", "roughness", "metalness", "clarity", "parts"} <= set(looks[0])
    pbr = m.render(size=(40, 30), colour="realistic")
    assert pbr.rgb.shape == (30, 40, 3)
    assert (pbr.rgb != m.render(size=(40, 30), colour="material").rgb).any()
    first = looks[0]["material"]
    m.set_materials({first: "copper roughness=0.2"})
    changed = {x["material"]: x for x in m.materials()}[first]
    assert changed["look"] == "copper" and changed["set"] and abs(changed["roughness"] - 0.2) < 1e-6
    with pytest.raises(ValueError):
        m.set_materials({first: "unobtainium"})
    m.set_materials(None)
    assert not {x["material"]: x for x in m.materials()}[first]["set"]
    # materials= draws one image with those looks, and leaves the model's.
    pink = {first: "colour=#ff00ff metalness=0"}
    # A slice through the middle of a cell of that material.
    cell = next(c.id for c in m.cells.values() if c.material == first and c.bbox)
    lo, hi = m.bbox(f"c {cell}")
    view = dict(z=(lo[2] + hi[2]) / 2, region=(lo, hi), width=60, colour="realistic", outline=False)
    once = m.slice(**view, materials=pink)
    assert not {x["material"]: x for x in m.materials()}[first]["set"]
    m.set_materials(pink)
    assert (once.rgb == m.slice(**view).rgb).all()
    assert (once.rgb != m.slice(**view, materials={}).rgb).any()
    m.set_materials(None)
    with pytest.raises(ValueError):
        m.render(size=(4, 4), materials={first: "unobtainium"})


def test_volumes_and_check():
    m = llevant.load(MODELS / "periodic.i")
    v = m.volumes(1.0, seed=2)
    groups = v.by("cell")
    assert sum(g.volume for g in groups) == pytest.approx(v.total)
    assert [g.number for g in groups if g.number is not None] == sorted(
        g.number for g in groups if g.number is not None
    )
    problems = m.check(1.0, seed=2)
    assert [p.name for p in problems] == [p.name for p in v.problems()]
    assert any(p.status == "overlap" and "(2, 9)" in p.name for p in problems)
    # The overlap is where cell 2 meets the graveyard (cell 9) at z = ±10:
    # a line along z there names both cells in its overlap labels.
    segs = m.trace((-3, -2, -12), (-3, -2, 12))
    # Groups that are one label carry it; numbered groups do not.
    assert all(p.label is not None and str(p.label) == p.name for p in problems)
    assert all(g.label is None for g in groups if g.number is not None)
    assert all(str(g.label) == g.name for g in v.by("label"))
    overlaps = [s.label for s in segs if s.label.status == "overlap"]
    assert overlaps and all(lab.involved == [2, 9] for lab in overlaps)
    assert all(s.label.involved == [] for s in segs if s.label.is_inside)


def test_box_volume_within_sigma():
    m = llevant.from_text(
        "box\n1 0 -1\n2 0 1 -2\n3 0 2\n\n1 rpp 0 2 0 3 0 4\n2 so 100\n\nnps 1\n"
    )
    assert m.path is None
    v = m.volumes(0.1, region=((-1, -1, -1), (3, 4, 5)))
    box = {g.number: g for g in v.by("cell")}[1]
    assert box.volume == pytest.approx(24.0, rel=5e-3)
    assert abs(box.volume - 24.0) < 5 * box.sigma
    assert len(box.by_probe) == box.probes_hit == 12 and box.h == 0.1
    assert box.converged is None
    groups = m.volumes_to(0.5, 1e-3, region=((-1, -1, -1), (3, 4, 5)))
    box = {g.number: g for g in groups}[1]
    assert box.converged and box.sigma <= 1e-3 * box.volume and box.h < 0.5
    assert box.outcome == "converged"
    assert box.volume == pytest.approx(24.0, rel=4e-3)
    # Every cell is listed: one in a universe nothing fills is empty.
    m = llevant.from_text(
        "box\n1 0 -1\n2 0 1 -2\n3 0 2\n5 1 -1.0 -3 u=7\n6 0 3 u=7\n\n"
        "1 rpp 0 2 0 3 0 4\n2 so 100\n3 so 1\n\nm1 1001.80c 1\nnps 1\n"
    )
    groups = {g.number: g for g in m.volumes_to(0.5, 1e-2, region=((-1, -1, -1), (3, 4, 5)))}
    assert groups[5].outcome == "empty" and groups[5].volume == 0.0
    # Chosen cells alone, in the order given.
    got = m.cell_volumes([5, 1], 0.5, 1e-2, region=((-1, -1, -1), (3, 4, 5)))
    assert [g.number for g in got] == [5, 1]
    assert got[0].outcome == "empty" and got[0].volume == 0.0
    assert got[1].outcome == "converged" and got[1].sigma <= 1e-2 * got[1].volume
    assert abs(got[1].volume - 24.0) < 4 * got[1].sigma
    with pytest.raises(KeyError):
        m.cell_volumes([10**9], 0.5)


def test_cells_and_info():
    m = llevant.load(MODELS / "universes.i")
    assert len(m.cells) == len(list(m.cells)) == m.info()["cells"]
    first = next(iter(m.cells))
    assert first in m.cells and m.cells[first].id == first
    assert m.cells.get(10**9) is None
    with pytest.raises(KeyError):
        m.cells[10**9]
    assert m.universes[0] == 0
    filled = [c for c in m.cells.values() if c.fills]
    assert filled and all(u in m.universes for c in filled for u in c.fills)
    assert "universes.i" in repr(m)


def test_instances():
    m = llevant.load(MODELS / "universes.i")
    assert m.instance_count > len(m.cells)
    lab = m.locate(152.3, 0.4, 0.3)
    assert lab.instance_path == "252 < 261[-3 1 0] < 15"
    assert m.instance(lab.instance_path) == lab.instance
    assert m.instance_path(lab.instance) == lab.instance_path
    assert m.instance_path(m.instance_count) is None
    with pytest.raises(ValueError):
        m.instance("252")
    r = m.classify([[152.3, 0.4, 0.3], [300.0, 0.0, 0.0]])  # the second on a surface
    assert list(r.instance) == [lab.instance, -1]


def test_boxes_and_isolated_renders():
    m = llevant.load(MODELS / "universes.i")
    # The cells of universe 1, at each of the places it is put.
    rows = m.instance_boxes("u 1")
    assert len(rows) > len({c for c, _, _ in rows})
    lo, hi = m.bbox("u 1")
    for _, (a, b), _ in rows:
        assert all(lo[k] <= a[k] and b[k] <= hi[k] for k in range(3))
    # Every point of the universe's cells, wherever they are, is in a box.
    ids = {c for c, _, _ in rows}
    pts = np.random.default_rng(3).uniform(*m.extent(), size=(20000, 3))
    r = m.classify(pts)
    for p, c in zip(pts, np.asarray(r.cell)):
        if c in ids:
            assert any(
                all(a[k] <= p[k] <= b[k] for k in range(3)) for cc, (a, b), _ in rows if cc == c
            )
    with pytest.raises(ValueError):
        m.bbox("c 99999")
    # Framed on what is isolated: more of the image is the cells.
    def hit(img):
        return (np.asarray(img.rgb) != 255).any(axis=-1).mean()

    # Envelope 10 (slot 0) and what fills it, placed once.
    framed = m.render(size=(64, 64), isolate="c 10")
    whole = m.render(size=(64, 64), isolate="c 10", region=m.extent())
    assert hit(framed) > 2 * hit(whole)
    assert hit(m.render(size=(64, 64), hide_rules=["all"])) == 0


def test_isolated_render_faces_a_plate(tmp_path):
    path = tmp_path / "plate.i"
    path.write_text(
        "plate\n1 3 -7.8 -1 imp:n=1\n2 0 1 -9 imp:n=1\n9 0 9 imp:n=0\n\n"
        "1 rpp -0.5 0.5 -20 20 -20 20\n9 so 100\n\nm3 26000 1\n"
    )
    m = llevant.load(path)

    def hit(img):
        return (np.asarray(img.rgb) != 255).any(axis=-1).mean()

    chosen = m.render(size=(48, 48), isolate="c 1")
    edge = m.render(size=(48, 48), isolate="c 1", direction=(0.0, 1.0, 0.0))
    assert hit(chosen) > 5 * hit(edge)


LABELLED = """labelled
1 0 -1 fill=1 imp:n=1 $ envelope A
2 0 -2 fill=1 (20 0 0) imp:n=1 $ envelope B
8 0 1 2 -9 imp:n=1
9 0 9 imp:n=0
10 3 -7.8 -4 u=1 imp:n=1 $ NB CELL - Steel ball
11 0 4 u=1 imp:n=1 $ NB CELL - void around

1 so 5
2 s 20 0 0 5
4 so 2
9 so 100

m3 26000 1
"""


def test_label_rules(tmp_path):
    path = tmp_path / "labelled.i"
    path.write_text(LABELLED)
    for m in [llevant.load(path), llevant.from_text(LABELLED)]:
        assert {c for c, _, _ in m.instance_boxes('label "steel"')} == {10}
        assert {c for c, _, _ in m.instance_boxes("$ nb cell & m 3")} == {10}
        assert {c for c, _, _ in m.instance_boxes("$ envelope")} == {1, 2}
        lo, hi = m.bbox("$ steel ball")
        assert lo[0] < 0 < 20 < hi[0]
        img = m.render(size=(32, 32), isolate="$ steel")
        assert (np.asarray(img.rgb) != 255).any()
        # A universe on its own keeps the numbers and the labels.
        u = m.universe(1)
        assert {c for c, _, _ in u.instance_boxes("$ nb cell")} == {10, 11}
    with pytest.raises(ValueError):
        llevant.load(path).bbox('label "unclosed')


def test_fields(tmp_path):
    m = llevant.load(MODELS / "universes.i")
    p = (152.3, 0.4, 0.3)
    lab = m.locate(*p)
    f = llevant.Field.cells(m, {lab.cell: 7.0}, name="heat", unit="W")
    assert (f.kind, f.name, f.unit) == ("cells", "heat", "W")
    assert f.value(*p) == 7.0
    assert np.isnan(f.values([p, (1e6, 0, 0)])[1])
    # Per instance: only this instance of the cell.
    inst = llevant.Field.instances(m, {lab.instance_path: 2.0})
    assert inst.value(p) == 2.0
    pts = np.array([[152.3 + d, 0.4, 0.3] for d in range(1, 30)])
    labels = m.classify(pts)
    k = next(k for k in range(len(pts)) if labels[k].cell == lab.cell and labels[k].instance != lab.instance)
    assert np.isnan(inst.values(pts)[k])
    # On a mesh: two voxels along x.
    mesh = llevant.Field.mesh(m, [[[1.0]], [[100.0]]], x=[0, 284, 568], y=[-20, 20], z=[-20, 20])
    assert mesh.kind == "mesh" and mesh.scale[2]
    assert mesh.value(p) == 1.0
    assert mesh.with_scale(0.5, 50).scale[:2] == (0.5, 50)
    with pytest.raises(ValueError):
        mesh.with_scale(10, 1)
    # From a CSV file, drawn over a slice and a render.
    csv = tmp_path / "f.csv"
    csv.write_text("material,heat [W]\n1,1\n2,1000\n")
    g = llevant.Field.read(m, csv)
    plain = m.slice(z=0.3, width=200)
    over = m.slice(z=0.3, width=200, field=g)
    assert plain.rgb.shape == over.rgb.shape and (plain.rgb != over.rgb).any()
    r = m.render(size=(60, 40), field=g.with_opacity(0.5))
    assert r.rgb.shape == (40, 60, 3)
    with pytest.raises(ValueError):
        llevant.load(MODELS / "surfaces.i").slice(z=0, field=g)


def test_mctal_fields():
    """aci_tallies' MCTAL probe: one ray through a lattice, F4 = chord / volume."""
    fixtures = Path(__file__).resolve().parents[3] / "tests" / "fixtures" / "tallies" / "mctal"
    m = llevant.load(fixtures / "probe.mcnp")
    mctal = fixtures / "probe.m"
    # Chains through lattice elements: a field per instance.
    f = llevant.Field.read(m, mctal, tally=24)
    assert f.kind == "instances" and f.name.startswith("tally 24 result")
    assert f.value(2.5, 0.3, 0.7) == pytest.approx(3.0 / 300.0, rel=1e-5)
    assert f.value(-30.0, 0.3, 0.7) is None
    # Three multiplier bins and no Total: one must be chosen.
    with pytest.raises(ValueError, match="m bin"):
        llevant.Field.read(m, mctal, tally=54)
    g = llevant.Field.read(m, mctal, tally=54, bins={"m": 1})
    assert g.kind == "cells" and g.unit == ""
    assert g.value(-30.0, 0.3, 0.7) == pytest.approx(2 * 20.0 / 8000.0, rel=1e-5)
    e = llevant.Field.read(m, mctal, tally=54, bins={"m": 0, "u": "total"}, error=True)
    assert e.name.startswith("tally 54 rel. error")
    with pytest.raises(ValueError):
        llevant.Field.read(m, mctal, tally=54, bins={"q": 1})
    with pytest.raises(ValueError, match="surface tallies"):
        llevant.Field.read(m, mctal, tally=2)


def test_universe_alone():
    m = llevant.load(MODELS / "universes.i")
    assert m.problem_box is not None and m.cut_cell is None
    u = m.universes[1]
    sub = m.universe(u)
    assert sub.universes[0] == u and sub.cut_cell in sub.cells
    lo, hi = sub.problem_box
    beyond = [hi[0] + 1.0, hi[1] + 1.0, hi[2] + 1.0]
    assert sub.locate(beyond).cell == sub.cut_cell
    assert sub.reload().universes[0] == u
    assert m.universe(u, cut="none").cut_cell is None
    box = m.universe(u, cut=(lo, hi))
    assert box.problem_box is not None
    with pytest.raises(ValueError):
        m.universe(u, cut="tight")


def test_recompile_text():
    text = (MODELS / "universes.i").read_text()
    m = llevant.from_text(text)
    # A cylinder moved.
    edited = text.replace("\n301 c/z 1 0 3\n", "\n301 c/z 1 0.2 3\n")
    assert edited != text
    r = m.recompile(edited)
    fresh = llevant.from_text(edited)
    assert r.path is None
    assert not r.diff(fresh)
    assert m.diff(r)
    pts = [[3.0 * k, 0.2 * k, -0.1 * k] for k in range(-60, 60)]
    assert list(r.classify(pts).cell) == list(fresh.classify(pts).cell)
    assert not m.diff(r.recompile(text))


def test_fingerprints_and_diff():
    m = llevant.load(MODELS / "universes.i")
    fp = m.fingerprints()
    assert set(fp.cells) == set(m.cells)
    d = m.diff(m.reload())
    assert not d and len(d.cells) == 0 and d.cells.unchanged == len(m.cells)
    other = llevant.load(MODELS / "surfaces.i")
    assert m.diff(other)


def test_errors():
    m = llevant.load(MODELS / "universes.i")
    with pytest.raises(OSError):
        llevant.load(MODELS / "missing.i")
    with pytest.raises(ValueError):
        m.slice()
    with pytest.raises(ValueError):
        m.classify([[1.0, 2.0]])
    with pytest.raises(ValueError):
        m.render(hide=[10**9])
    with pytest.raises(ValueError):
        m.trace_lines([[0, 0, 0]], [[0, 0, 0]])
    with pytest.raises(ValueError):
        m.volumes(0.0)
    with pytest.raises(ValueError):
        m.slice(z=0, colour="rainbow")


def test_frames():
    m = llevant.load(MODELS / "universes.i")
    pts, cells = golden("universes")
    r = m.classify(pts[:500])
    h = m.first_hit(pts[:50], np.tile([0.0, 0.0, 1.0], (50, 1)), t_max=1.0)
    pl = pytest.importorskip("polars")
    df = r.to_polars()
    assert df.shape[0] == 500 and df["cell"].to_list() == list(cells[:500])
    assert df["label"].cast(pl.String).to_list() == [str(r[k]) for k in range(500)]
    hf = h.to_polars()
    assert hf["label"].null_count() == int((~np.asarray(h.hit)).sum())
    # `to_numpy()` of a frame is Fortran-ordered.
    xyz = pl.DataFrame(pts[:500], schema=["x", "y", "z"], orient="row").to_numpy()
    assert xyz.flags.f_contiguous
    np.testing.assert_array_equal(m.classify(xyz).cell, cells[:500])
    pd = pytest.importorskip("pandas")
    pf = m.trace_lines(pts[:20], pts[20:40]).to_pandas()
    assert list(pf.columns[:3]) == ["label", "status", "line"]
    assert isinstance(m.volumes(4.0).to_pandas("material"), pd.DataFrame)


def test_tour_runs(tmp_path):
    """The example in examples/tour.py keeps working."""
    import subprocess
    import sys

    tour = Path(__file__).resolve().parents[1] / "examples" / "tour.py"
    model = MODELS / "universes.i"
    run = subprocess.run(
        [sys.executable, str(tour), str(model), str(tmp_path)],
        capture_output=True,
        text=True,
    )
    assert run.returncode == 0, run.stderr
    assert (tmp_path / "slice.png").exists() and (tmp_path / "cutaway.png").exists()


def test_stubs_match_the_module():
    """python/llevant/_llevant.pyi describes what the compiled module has."""
    import subprocess
    import sys

    pytest.importorskip("mypy")
    allow = Path(__file__).with_name("stubtest_allowlist.txt")
    run = subprocess.run(
        [sys.executable, "-m", "mypy.stubtest", "llevant", "--allowlist", str(allow)],
        capture_output=True,
        text=True,
        cwd=Path(__file__).resolve().parents[1],
    )
    assert run.returncode == 0, run.stdout + run.stderr


def test_levels_and_ghosts():
    """Paths down to a level, MCNP's LEVEL n in views, ghost envelopes."""
    m = llevant.load(MODELS / "universes.i")
    lo, hi = m.extent()
    rng = np.random.default_rng(5)
    pts = rng.uniform(lo, hi, size=(2000, 3))
    full = m.classify(pts)
    top = m.classify(pts, level=0)
    for k in range(0, len(pts), 37):
        a, b = full[k], top[k]
        assert len(b.path) <= 1
        if a.is_inside and len(a.path) > 1:
            assert b.path == a.path[:1] and b.is_inside
        assert m.locate(*pts[k], level=0) == b
    # Rays along the row of envelopes: at level 0 every hit is a level-0
    # cell; ghost envelopes are crossed, what fills them is hit.
    start = np.tile([-20.0, 0.0, 0.0], (50, 1)) + rng.uniform(-1, 1, size=(50, 3))
    d = np.tile([1.0, 0.0, 0.0], (50, 1))
    h0 = m.first_hit(start, d, level=0)
    for k in np.flatnonzero(h0.hit):
        assert h0[k].status != "inside" or len(h0[k].path) == 1
    envelopes = [c for c in m.cells.values() if c.fills and c.universe == 0]
    rule = "c " + ",".join(str(c.id) for c in envelopes[:5])
    hg = m.first_hit(start, d, ghost=[rule])
    assert (np.asarray(hg.ghost_count) > 0).any()
    ids = {c.id for c in envelopes[:5]}
    crossed = np.asarray(hg.ghost_cell)[np.asarray(hg.ghost_count) > 0]
    assert set(crossed.tolist()) <= ids
    assert (np.asarray(m.first_hit(start, d).ghost_count) == 0).all()
    img = m.slice(z=0.0, width=200, level=1)
    assert img.width == 200
    # Above level 1 (the level-0 cells not filled) is blank (white), though
    # the labels still say where each pixel is.
    assert (np.asarray(img.rgb) == 255).all(axis=-1).any()
    full_img = m.slice(z=0.0, width=200)
    assert not (np.asarray(full_img.rgb) == 255).all(axis=-1).any()
    m.render(size=(64, 48), ghost=[rule], level=None)


def _elf_dynamic_flags(path):
    """DT_FLAGS of a 64-bit little-endian ELF shared object (0 if absent)."""
    import struct

    data = Path(path).read_bytes()
    assert data[:4] == b"\x7fELF" and data[4] == 2 and data[5] == 1
    phoff, = struct.unpack_from("<Q", data, 0x20)
    phentsize, phnum = struct.unpack_from("<HH", data, 0x36)
    for k in range(phnum):
        p_type, _, p_offset, _, _, p_filesz = struct.unpack_from(
            "<IIQQQQ", data, phoff + k * phentsize
        )
        if p_type == 2:  # PT_DYNAMIC
            for off in range(p_offset, p_offset + p_filesz, 16):
                tag, val = struct.unpack_from("<qQ", data, off)
                if tag == 0:
                    break
                if tag == 30:  # DT_FLAGS
                    return val
    return 0


@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="ELF only")
def test_no_static_tls():
    # A module loaded with dlopen that needs static TLS only loads while
    # glibc's small surplus lasts: `import llevant` failed after numpy and
    # others ("cannot allocate memory in static TLS block"). mimalloc must be
    # built with local-dynamic TLS (docs/decisions/0009-mimalloc.md).
    DF_STATIC_TLS = 0x10
    assert not _elf_dynamic_flags(llevant._llevant.__file__) & DF_STATIC_TLS


OVERLAPPING = """overlap
1 0 -1 imp:n=1 $ ball
2 0 -2 imp:n=1
3 0 1 2 -3 imp:n=0
4 0 3 imp:n=0

1 so 5
2 sx 4 5
3 so 50

"""


def overlaps(text):
    m = llevant.from_text(text)
    return [g for g in m.volumes(h=1.0).problems() if g.status == "overlap"]


def test_deck_cut_fixes_an_overlap():
    assert overlaps(OVERLAPPING)
    deck = llevant.Deck.from_text(OVERLAPPING)
    cut = deck.cut(2, 1)
    assert cut.keep == 2 and cut.cut == 1
    assert "#1" in cut.new_card
    assert cut.terms_after <= cut.terms_before
    assert not cut.empty
    deck.apply(cut)
    assert not overlaps(deck.text())
    assert "$ ball" in deck.text()  # nothing else changed
    assert deck.undo()
    assert deck.text() == OVERLAPPING
    assert not deck.undo()


def test_deck_cut_is_simplified_by_boolean_algebra():
    deck = llevant.Deck.from_text(
        "t\n1 0 (-1:-2) 3 imp:n=1\n2 0 -1 3 imp:n=1\n\n1 so 5\n2 so 9\n3 px 0\n\n"
    )
    plain = deck.cut(1, 2, simplify=False)
    cut = deck.cut(1, 2)
    assert plain.geometry == "(-1:-2) 3 #2" and not plain.simplified
    # The same three terms, in whatever order the algebra writes them.
    assert sorted(cut.geometry.split()) == ["-2", "1", "3"] and cut.simplified
    assert cut.trace


def test_deck_cut_refuses_what_it_cannot_do(tmp_path):
    deck = llevant.Deck.from_text(OVERLAPPING)
    with pytest.raises(ValueError, match="no cell 9"):
        deck.cut(1, 9)
    inside = llevant.Deck.from_text("t\n1 0 -1 imp:n=1\n2 0 -1 2 imp:n=1\n\n1 so 5\n2 px 0\n\n")
    empty = inside.cut(2, 1)
    assert empty.empty
    with pytest.raises(ValueError, match="inside"):
        inside.apply(empty)
    with pytest.raises(ValueError, match="no file"):
        deck.save()
    out = tmp_path / "fixed.i"
    deck.apply(deck.cut(2, 1))
    deck.save(out)
    assert llevant.Deck.open(out).text() == deck.text()


def test_deck_does_not_save_a_file_that_is_not_utf8(tmp_path):
    # A Latin-1 `é` in the title: writing the text read would change it.
    path = tmp_path / "latin1.i"
    raw = OVERLAPPING.replace("\n", " caf\xe9\n", 1).encode("latin-1")
    path.write_bytes(raw)
    deck = llevant.Deck.open(path)
    with pytest.raises(ValueError, match="not UTF-8"):
        deck.save()
    with pytest.raises(ValueError, match="not UTF-8"):
        deck.save(tmp_path / "other.i")
    assert path.read_bytes() == raw


def test_deck_refuses_a_cut_planned_before_another_even_of_another_cell():
    deck = llevant.Deck.from_text(OVERLAPPING)
    first = deck.cut(2, 1)
    second = deck.cut(1, 2)
    deck.apply(first)
    with pytest.raises(ValueError, match="changed since"):
        deck.apply(second)
    # Planned again after the undo, it is written.
    deck.undo()
    deck.apply(deck.cut(1, 2))
    assert "#2" in deck.text()


def test_deck_refuses_cells_that_would_complement_each_other():
    deck = llevant.Deck.from_text(
        "t\n1 0 -1 imp:n=1\n2 0 #1 : -2 imp:n=1\n\n1 so 5\n2 so 9\n\n"
    )
    with pytest.raises(ValueError, match="complement each other"):
        deck.cut(1, 2)


def test_deck_refuses_a_cut_cell_that_is_a_like_cell():
    deck = llevant.Deck.from_text(
        "t\n1 0 -1 imp:n=1\n2 like 3 but imp:n=2\n3 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n\n"
    )
    with pytest.raises(ValueError, match="LIKE"):
        deck.cut(1, 2)


def test_deck_refuses_a_cut_planned_before_another():
    deck = llevant.Deck.from_text(OVERLAPPING)
    first = deck.cut(2, 1)
    second = deck.cut(2, 3)
    deck.apply(first)
    with pytest.raises(ValueError, match="changed since"):
        deck.apply(second)
    assert "#1" in deck.text()
