#!/usr/bin/env python3
"""Build the transport-neutral fixture from the preserved offline snapshots."""
import copy
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "fixtures/catalog"
snapshot = json.loads((ROOT / "tmdb.json").read_text())
manifest = json.loads((ROOT / "artwork.json").read_text())
titles = json.loads((ROOT / "titles.json").read_text())


def art(record):
    if not record:
        return None
    file = record.get("renderFile", record["file"])
    assert (ROOT / "artwork" / record["file"]).is_file()
    assert (ROOT / "artwork" / file).is_file()
    return dict(id=record["id"], url="/artwork/" + file, label=record["label"],
                creator=record["creator"], license=record["license"], source=record["source"],
                license_url=record["licenseUrl"], kind=record["type"])


items, credits = [], {}
runtime = [5640, 4740, 5520, 5760, 600, 888, 734, 630, None]
for n, title in enumerate(titles):
    record = snapshot["items"].get(title["id"], {})
    items.append(dict(id=title["id"], title=title["title"], kind="Movie" if n < 8 else "Series",
        parent_id=None, year=title["year"], subtitle=title["meta"], overview=record.get("overview", ""),
        tags=title["tags"].split(), colors=title["colors"], release_date=record.get("date", ""),
        added_order=n+1, title_rights=dict(label=title["rights"], note=title["note"],
            source=title["source"], license_url=title["license"], attribution=title["attribution"]),
        metadata_rights=dict(label="TMDB API metadata", note="This product uses the TMDB API but is not endorsed or certified by TMDB.",
            source="https://www.themoviedb.org", license_url="https://developer.themoviedb.org/docs/faq", attribution="The Movie Database (TMDB)"),
        artwork=art(next((a for a in manifest if a["id"] == title["id"]), None)),
        rating=None, runtime=dict(seconds=runtime[n], minimum_seconds=3780 if n == 0 else None,
            maximum_seconds=5640 if n == 0 else None, note="version dependent" if n < 2 else "", display=title["meta"]),
        provider_references=[], progress_percent=None, provenance_notes=[], metadata_retrieved_at=""))

series = items[-1]
season = copy.deepcopy(series)
season.update(id="beverly-hillbillies-s1", title="Season 1", kind="Season", parent_id=series["id"],
    subtitle="2 included episodes", overview="Selected episodes from the 36-episode first season; not the complete season.")
items.append(season)
for n, title in enumerate(["The Clampetts Strike Oil", "Getting Settled"]):
    episode = copy.deepcopy(series)
    episode.update(id=f"beverly-hillbillies-s1-e{n+1}", title=title, kind="Episode", parent_id=season["id"],
        subtitle=f"The Beverly Hillbillies · S1 E{n+1} · ~25m", added_order=10+n,
        runtime=dict(seconds=1500, minimum_seconds=None, maximum_seconds=None, note="approximately", display="~25m"),
        progress_percent=42 if n == 0 else None)
    items.append(episode)

for item in items:
    record = snapshot["items"].get(item["id"], {})
    if record:
        item["metadata_retrieved_at"] = snapshot["retrievedAt"]
        item["provenance_notes"] = [record["dateNote"]] if record.get("dateNote") else []
        if item["id"] == "sprite-fright":
            item["provenance_notes"].append("TMDB title 891761 was verified through director Matthew Luhn and the official Blender film homepage, not a loose title/year match.")
        item["overview"] = record["overview"] or item["overview"]
        item["release_date"] = record["date"]
        item["provider_references"] = [dict(provider="TMDB", status="Listed", url=record["url"])]
        if record.get("rating") is not None:
            item["rating"] = dict(provider="TMDB", value=record["rating"], scale=10,
                                  votes=record.get("votes"), retrieved_at=snapshot["retrievedAt"])
    item["provider_references"].append(dict(provider="Jellyfin", status="Not connected", url=""))
    credits[item["id"]] = []
    for department in ["cast", "creators", "crew"]:
        for person in record.get(department, []):
            portrait = next((a for a in manifest if a.get("tmdbPersonId") == person["id"] or a["id"] == person["name"]), None)
            credits[item["id"]].append(dict(id=f"tmdb:{person['id']}", name=person["name"],
                role=person["role"], department=department, artwork=art(portrait)))

home = [
    dict(id="continue-up-next", title="Continue Watching / Up Next", ids=["big-buck-bunny", "beverly-hillbillies-s1-e1", "his-girl-friday", "beverly-hillbillies-s1-e2", "sintel", "tears-of-steel"]),
    dict(id="released-movies", title="Recently Released Movies", ids=[i["id"] for i in sorted(items[:8], key=lambda i:i["release_date"], reverse=True)]),
    dict(id="added-movies", title="Recently Added Movies", ids=[i["id"] for i in reversed(items[:8])]),
    dict(id="released-episodes", title="Recently Released Episodes", ids=[items[-1]["id"], items[-2]["id"]]),
    dict(id="added-episodes", title="Recently Added Episodes", ids=[items[-1]["id"], items[-2]["id"]]),
]
info = dict(artwork=[art(record) for record in manifest],
            notices=json.loads((ROOT / "notices.json").read_text()), retrieved_at=snapshot["retrievedAt"])
(ROOT / "fixture.json").write_text(json.dumps(dict(items=items, credits=credits, home=home, info=info), ensure_ascii=False, indent=2) + "\n")
