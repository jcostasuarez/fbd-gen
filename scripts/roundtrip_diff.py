"""Compara una sección generada (.xbd) con lo que reexporta Control Expert.

    python scripts/roundtrip_diff.py out/Planta_Bombas/manifest.json [--reexport-dir out/ce_test]

Usa los <sección>.reexport.xml que deja `ce_check.py --dump`. Compara bloques (tipo,
posición, tamaño, pines en orden con sus parámetros), links (extremos y quiebres) y
textBox (texto, posición, tamaño). Ignora encabezados y atributos que agrega el
exportador (fechas, SectionOrder). Sale con código 1 si hay diferencias.
"""
import json
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def model(path):
    root = ET.parse(path).getroot()
    net = root.find(".//networkFBD")
    blocks, links, texts = {}, [], []
    for b in net.findall("FFBBlock"):
        pos = b.find("objPosition")
        pins = [
            (p.tag, p.get("formalParameter"), p.get("effectiveParameter"), p.get("invertedPin"))
            for p in b.find("descriptionFFB")
        ]
        comment = b.find("comment")
        blocks[b.get("instanceName")] = {
            "type": b.get("typeName"),
            "pos": (pos.get("posX"), pos.get("posY")),
            "size": (b.get("width"), b.get("height")),
            "extra": b.get("additionnalPinNumber"),
            "pins": pins,
            "comment": comment.text if comment is not None else None,
        }
    for l in net.findall("linkFB"):
        end = lambda tag: (l.find(tag).get("parentObjectName"), l.find(tag).get("pinName"),
                           l.find(tag + "/objPosition").get("posX"), l.find(tag + "/objPosition").get("posY"))
        bends = [(g.get("posX"), g.get("posY")) for g in l.findall("gridObjPosition")]
        links.append((end("linkSource"), end("linkDestination"), tuple(bends)))
    for t in net.findall("textBox"):
        pos = t.find("objPosition")
        texts.append(((t.text or "").strip(), (pos.get("posX"), pos.get("posY")), (t.get("width"), t.get("height"))))
    variables = {(v.get("name"), v.get("typeName")) for v in root.findall(".//dataBlock/variables")}
    return blocks, sorted(links), sorted(texts), variables


def diff(name, ours, theirs):
    problems = []
    (b1, l1, t1, v1), (b2, l2, t2, _) = ours, theirs
    for inst in sorted(set(b1) | set(b2)):
        if inst not in b2:
            problems.append(f"bloque {inst}: falta en el reexport")
        elif inst not in b1:
            problems.append(f"bloque {inst}: aparece solo en el reexport")
        else:
            for key in ("type", "pos", "size", "extra", "pins", "comment"):
                if b1[inst][key] != b2[inst][key]:
                    problems.append(f"bloque {inst}.{key}: {b1[inst][key]} != {b2[inst][key]}")
    if l1 != l2:
        for l in set(l1) - set(l2):
            problems.append(f"link solo generado: {l}")
        for l in set(l2) - set(l1):
            problems.append(f"link solo en reexport: {l}")
    if t1 != t2:
        problems.append(f"textBox: {t1} != {t2}")
    print(f"{name}: {len(b1)} bloques, {len(l1)} links, {len(t1)} textBox, {len(v1)} variables -> "
          + ("idéntico" if not problems else f"{len(problems)} diferencias"))
    for p in problems:
        print("   ", p)
    return not problems


def main():
    args = sys.argv[1:]
    redir = Path("out/ce_test")
    if "--reexport-dir" in args:
        i = args.index("--reexport-dir")
        redir = Path(args[i + 1])
        del args[i : i + 2]
    ok = True
    for arg in args:
        manifest = Path(arg)
        m = json.loads(manifest.read_text(encoding="utf-8"))
        for s in m["sections"]:
            ok &= diff(s["name"], model(manifest.parent / s["xml"]), model(redir / f"{s['name']}.reexport.xml"))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
