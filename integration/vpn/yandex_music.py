#!/usr/bin/env python3
"""Print a direct-routing rule for Yandex Music in the selected client format."""
import argparse
import base64
import json
from pathlib import Path

RULES = json.loads(Path(__file__).with_name("music-domains.json").read_text())
SUFFIXES, HOSTS = RULES["suffixes"], RULES["hosts"]


def xray_domains():
    return [f"domain:{name}" for name in SUFFIXES] + [f"full:{name}" for name in HOSTS]


def render(format_name, outbound):
    if format_name == "xray":
        return json.dumps({"type": "field", "domain": xray_domains(),
                           "outboundTag": outbound or "direct"}, indent=2)
    if format_name == "sing-box":
        return json.dumps({"domain_suffix": SUFFIXES, "domain": HOSTS,
                           "action": "route", "outbound": outbound or "direct"}, indent=2)
    if format_name == "mihomo":
        target = outbound or "DIRECT"
        return "\n".join([f"- DOMAIN-SUFFIX,{name},{target}" for name in SUFFIXES]
                         + [f"- DOMAIN,{name},{target}" for name in HOSTS])
    if format_name == "domains":
        return "\n".join((*SUFFIXES, *HOSTS))
    profile = {"Name": "Yandex Music DIRECT", "GlobalProxy": "true",
               "DirectSites": xray_domains()}
    if format_name == "happ-link":
        data = json.dumps(profile, separators=(",", ":")).encode()
        return "happ://routing/add/" + base64.b64encode(data).decode()
    return json.dumps(profile, indent=2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("format", choices=("xray", "sing-box", "mihomo", "happ", "happ-link", "domains"))
    parser.add_argument("--outbound", help="name of the direct outbound or policy")
    options = parser.parse_args()
    if options.outbound and options.format not in ("xray", "sing-box", "mihomo"):
        parser.error("--outbound applies to xray, sing-box, or mihomo")
    print(render(options.format, options.outbound))


if __name__ == "__main__":
    main()
