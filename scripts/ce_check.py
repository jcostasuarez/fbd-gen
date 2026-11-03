"""Verificación real en Control Expert vía el MCP de sce_mcp.

Crea un proyecto nuevo, importa una o más secciones FBD (.xbd generados por fbd-gen),
compila con rebuild completo y guarda el proyecto para abrirlo a mano si hace falta.

    python scripts/ce_check.py out/Planta_Bombas/manifest.json
    python scripts/ce_check.py out/P101_Motor/P101_Motor.xbd [más .xbd...]

Con un manifest.json las secciones se importan en el orden de ejecución que calculó el
pipeline (el orden de importación es el orden de las secciones en la tarea).

Con --dump, guarda cómo reexporta Control Expert cada sección importada
(out/ce_test/<sección>.reexport.xml), para comparar contra lo generado.

Variables de entorno:
    CE_MCP_EXE   ruta a control-expert-mcp.exe (default: ../sce_mcp/target/release/...)
    CE_CPU       part number de la CPU (default 'BME P58 2040')
    CE_CPU_VER   versión de firmware (default '03.20')
"""
import json
import os
import time
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = os.environ.get(
    "CE_MCP_EXE", str(ROOT.parent / "sce_mcp" / "target" / "release" / "control-expert-mcp.exe")
)


class Mcp:
    def __init__(self):
        self.p = subprocess.Popen([EXE], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, encoding="utf-8")
        self.id = 0
        self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "fbd-gen", "version": "0.1"}})
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _send(self, msg):
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()

    def request(self, method, params):
        self.id += 1
        self._send({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise SystemExit("el servidor MCP cerró la conexión")
            msg = json.loads(line)
            if msg.get("id") == self.id:
                return msg

    def call(self, name, **arguments):
        res = self.request("tools/call", {"name": name, "arguments": arguments})["result"]
        text = "\n".join(c["text"] for c in res.get("content", []) if c.get("type") == "text")
        try:
            data = json.loads(text)
        except ValueError:
            data = text
        if res.get("isError"):
            raise SystemExit(f"{name} falló: {text}")
        return data

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=60)


def main():
    files = []
    task = "MAST"
    args = sys.argv[1:]
    dump = "--dump" in args
    args = [a for a in args if a != "--dump"]
    for arg in args:
        path = Path(arg).resolve()
        if path.name == "manifest.json":
            manifest = json.loads(path.read_text(encoding="utf-8"))
            task = manifest.get("task", task)
            files += [path.parent / s["xml"] for s in manifest["sections"]]
        else:
            files.append(path)
    if not files:
        raise SystemExit(__doc__)
    mcp = Mcp()
    ok = True
    try:
        for f in files:
            report = mcp.call("validate_xml", file_path=str(f))
            for e in report.get("errors", []):
                print(f"[validate_xml] {f.name}: {e}")
        mcp.call("new_project", cpu_part_number=os.environ.get("CE_CPU", "BME P58 2040"),
                 cpu_version=os.environ.get("CE_CPU_VER", "03.20"))
        for f in files:
            mcp.call("import_xml", kind="section", task=task, file_path=str(f))
            print(f"importada: {f.name}")
        build = mcp.call("build_project", rebuild_all=True)
        print(build["output"] if isinstance(build, dict) else build)
        ok = isinstance(build, dict) and build.get("build_state") == "built_ok"
        structure = mcp.call("get_project_structure")
        for t in structure.get("tasks", []):
            print(f"{t['name']}: " + " -> ".join(s["name"] for s in t.get("sections", [])))
        # Nombre por corrida: Control Expert puede dejar bloqueado el .stu de la anterior.
        out = ROOT / "out" / "ce_test" / time.strftime("fbd_check_%Y%m%d_%H%M%S.stu")
        out.parent.mkdir(parents=True, exist_ok=True)
        if dump:
            for t in structure.get("tasks", []):
                for s in t.get("sections", []):
                    xml = mcp.call("read_section", section=s["name"], task=t["name"])
                    if isinstance(xml, dict):
                        xml = xml.get("xml") or json.dumps(xml, ensure_ascii=False, indent=1)
                    path = out.parent / f"{s['name']}.reexport.xml"
                    path.write_text(xml, encoding="utf-8")
                    print(f"reexport: {path}")
        mcp.call("save_project", path=str(out))
        print(f"proyecto guardado: {out}")
        mcp.call("close_project", save=False)
    finally:
        mcp.close()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
