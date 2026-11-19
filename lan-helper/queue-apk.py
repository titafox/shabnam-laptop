"""Queue a single signed installation request for this Mac's foreground helper."""
from pathlib import Path
import hashlib, sys, subprocess, uuid, base64, tempfile, os
root = Path(__file__).resolve().parent
dist = root / "dist"
source = Path(sys.argv[1])
if not 0 < source.stat().st_size <= 256 * 1024**2:
    sys.exit("APK size must be at most 256 MiB")
with tempfile.NamedTemporaryFile(dir=dist, delete=False) as package:
    digest = hashlib.sha256()
    with source.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
            package.write(chunk)
name = digest.hexdigest()
os.replace(package.name, dist / (name + ".apk"))
request = f"{uuid.uuid4().hex} install {name}"
signature = subprocess.check_output(["openssl","dgst","-sha256","-sign",str(root/".install-keys/private.pem")],input=request.encode())
with tempfile.NamedTemporaryFile(dir=dist, delete=False) as command:
    command.write(request.encode()+b" "+base64.b64encode(signature)+b"\n")
os.replace(command.name,dist/"diagnostic-action.txt")
