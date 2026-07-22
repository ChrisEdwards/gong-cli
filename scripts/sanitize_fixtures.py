#!/usr/bin/env python3
"""Create deterministic, publishable Gong fixtures from ignored raw spikes."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable, Iterator


FIXTURE_NAMES = (
    "extensive_response.json",
    "retention_response.json",
    "january_response.json",
    "transcript_response.json",
)
REFERENCE_CALL_ID = "1860496513693944597"
SALT = b"gong-cli-public-fixtures-v1\0"
TIMESTAMP = re.compile(r"^\d{4}-\d{2}-\d{2}T")
EMAIL = re.compile(r"(?i)\b[A-Z0-9._%+-]+@([A-Z0-9.-]+\.[A-Z]{2,})\b")
MONEY_FIELD = re.compile(r"(?i)(arr|amount|revenue|value|price|cost)")
HOSTILE_TITLE_CHARS = '<>:"/\\|?*'


class SanitizationError(RuntimeError):
    pass


def digest(namespace: str, value: Any, length: int = 10) -> str:
    payload = f"{namespace}\0{value}".encode("utf-8")
    return hashlib.sha256(SALT + payload).hexdigest()[:length]


def numeric_identifier(namespace: str, value: Any) -> str:
    original = str(value)
    width = max(len(original), 6)
    number = int(hashlib.sha256(SALT + f"{namespace}\0{original}".encode()).hexdigest(), 16)
    digits = str(number % (10**width)).zfill(width)
    if digits[0] == "0":
        digits = "7" + digits[1:]
    return digits


def iter_strings(value: Any, path: tuple[Any, ...] = ()) -> Iterator[tuple[tuple[Any, ...], str]]:
    if isinstance(value, dict):
        for key, child in value.items():
            yield from iter_strings(child, path + (key,))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from iter_strings(child, path + (index,))
    elif isinstance(value, str):
        yield path, value


class Sanitizer:
    def sanitize_document(self, document: dict[str, Any]) -> dict[str, Any]:
        result = copy.deepcopy(document)
        if "calls" in result:
            result["calls"] = [self.sanitize_call(call) for call in result["calls"]]
        if "callTranscripts" in result:
            result["callTranscripts"] = [
                self.sanitize_transcript(item) for item in result["callTranscripts"]
            ]

        for key in list(result):
            if key not in {"calls", "callTranscripts"}:
                result[key] = self.sanitize_unknown(result[key], (key,))
        return result

    def sanitize_call(self, call: dict[str, Any]) -> dict[str, Any]:
        result = copy.deepcopy(call)
        metadata = result.get("metaData", {})
        call_id = str(metadata.get("id", "unknown"))
        self.sanitize_metadata(metadata, call_id)

        original_parties = result.get("parties", [])
        result["parties"] = [self.sanitize_party(party) for party in original_parties]
        phone_party_indexes = [
            index
            for index, party in enumerate(original_parties)
            if party.get("affiliation") == "Unknown" and not party.get("emailAddress")
        ]
        if len(phone_party_indexes) >= 5 and any(
            party.get("affiliation") == "Internal" for party in original_parties
        ):
            for index in phone_party_indexes:
                result["parties"][index]["name"] = None
                result["parties"][index]["emailAddress"] = None
        if "content" in result:
            result["content"] = self.sanitize_content(result["content"])
        if "context" in result:
            result["context"] = self.sanitize_crm_context(result["context"])
        if "interaction" in result:
            result["interaction"] = self.sanitize_interaction(result["interaction"])

        for key in list(result):
            if key not in {"metaData", "parties", "content", "context", "interaction"}:
                result[key] = self.sanitize_unknown(result[key], ("call", key))
        return result

    def sanitize_metadata(self, metadata: dict[str, Any], call_id: str) -> None:
        original = copy.deepcopy(metadata)
        for key, value in original.items():
            if value is None or isinstance(value, (bool, int, float)):
                continue
            if key == "id":
                metadata[key] = call_id
            elif key == "title":
                metadata[key] = self.sanitize_title(value)
            elif key in {"scheduled", "started"}:
                metadata[key] = value
            elif key in {"primaryUserId", "workspaceId", "calendarEventId"}:
                metadata[key] = numeric_identifier("identity-id", value)
            elif key == "url":
                metadata[key] = f"https://example.gong.io/call/{call_id}"
            elif key == "meetingUrl":
                metadata[key] = f"https://meet.example/{digest('meeting-url', value)}"
            elif key in {"direction", "system", "scope", "media", "language"}:
                metadata[key] = value
            else:
                metadata[key] = self.sanitize_unknown(value, ("metaData", key))

    def sanitize_title(self, original: str) -> str:
        result = f"Fixture_{digest('call-title', original)}"
        present = "".join(character for character in HOSTILE_TITLE_CHARS if character in original)
        if present:
            result += f" {present}"
        if "“" in original or "”" in original:
            result += " “Quoted”"
        if re.search(r"\s{2,}", original):
            result += "  Spaced"
        return result

    def sanitize_party(self, party: dict[str, Any]) -> dict[str, Any]:
        result = copy.deepcopy(party)
        affiliation = party.get("affiliation")
        identity = party.get("emailAddress") or party.get("name") or party.get("id") or "unknown"

        for key, value in party.items():
            if value is None:
                continue
            if key in {"id", "userId", "speakerId"}:
                result[key] = numeric_identifier("identity-id", value)
            elif key == "name":
                result[key] = f"P_{digest('person-name', value)}"
            elif key == "emailAddress":
                domain = value.rsplit("@", 1)[-1].casefold() if "@" in value else value.casefold()
                if affiliation == "Internal":
                    fake_domain = "internal-example.com"
                else:
                    fake_domain = f"customer-{digest('email-domain', domain, 8)}.example"
                result[key] = f"p-{digest('email-address', identity)}@{fake_domain}"
            elif key == "title":
                result[key] = f"R_{digest('job-title', value)}"
            elif key == "phoneNumber":
                suffix = int(digest("phone", value, 6), 16) % 10000
                result[key] = f"+1-555-{suffix:04d}"
            elif key == "affiliation":
                result[key] = value
            else:
                result[key] = self.sanitize_unknown(value, ("party", key))
        return result

    def sanitize_content(self, content: Any) -> Any:
        if not isinstance(content, dict):
            return self.sanitize_unknown(content, ("content",))
        result = copy.deepcopy(content)

        if isinstance(result.get("brief"), str):
            result["brief"] = f"B_{digest('brief', result['brief'])}."

        for index, point in enumerate(result.get("keyPoints") or []):
            if isinstance(point, dict) and isinstance(point.get("text"), str):
                point["text"] = f"K_{digest('key-point', point['text'])}."
            result["keyPoints"][index] = self.sanitize_known_text_container(point, "key-point")

        for highlight in result.get("highlights") or []:
            if isinstance(highlight.get("title"), str):
                title = highlight["title"]
                highlight["title"] = "Next steps" if title.casefold() == "next steps" else f"H_{digest('highlight-title', title)}"
            for item in highlight.get("items") or []:
                if isinstance(item.get("text"), str):
                    item["text"] = f"N_{digest('highlight-item', item['text'])}."

        for section in result.get("outline") or []:
            if isinstance(section.get("section"), str):
                section["section"] = f"S_{digest('outline-section', section['section'])}"
            for item in section.get("items") or []:
                if isinstance(item.get("text"), str):
                    item["text"] = f"O_{digest('outline-item', item['text'])}."

        for topic in result.get("topics") or []:
            if isinstance(topic, dict) and isinstance(topic.get("name"), str):
                topic["name"] = f"T_{digest('topic', topic['name'])}"

        for key in list(result):
            if key not in {"brief", "keyPoints", "highlights", "outline", "topics"}:
                result[key] = self.sanitize_unknown(result[key], ("content", key))
        return result

    def sanitize_known_text_container(self, value: Any, namespace: str) -> Any:
        if not isinstance(value, dict):
            return self.sanitize_unknown(value, (namespace,))
        result = copy.deepcopy(value)
        for key in result:
            if key != "text":
                result[key] = self.sanitize_unknown(result[key], (namespace, key))
        return result

    def sanitize_crm_context(self, contexts: Any) -> Any:
        if not isinstance(contexts, list):
            return self.sanitize_unknown(contexts, ("context",))
        result = copy.deepcopy(contexts)
        for context in result:
            if not isinstance(context, dict):
                continue
            if "system" in context:
                context["system"] = "CRM"
            for obj in context.get("objects") or []:
                object_type = obj.get("objectType")
                if isinstance(object_type, str):
                    obj["objectType"] = object_type if object_type == "Account" else "CRMObject"
                if isinstance(obj.get("objectId"), str):
                    obj["objectId"] = numeric_identifier("crm-object-id", obj["objectId"])
                for field in obj.get("fields") or []:
                    field_name = field.get("name")
                    if "value" in field and field["value"] is not None:
                        if object_type == "Account" and field_name == "Name":
                            field["value"] = f"A_{digest('account-name', field['value'])}"
                        else:
                            field["value"] = f"V_{digest('crm-value', field['value'])}"
        return result

    def sanitize_interaction(self, interaction: Any) -> Any:
        if not isinstance(interaction, dict):
            return self.sanitize_unknown(interaction, ("interaction",))
        result = copy.deepcopy(interaction)
        for speaker in result.get("speakers") or []:
            for key in ("id", "userId", "speakerId"):
                if key in speaker and speaker[key] is not None:
                    speaker[key] = numeric_identifier("identity-id", speaker[key])
        for key in list(result):
            if key != "speakers":
                result[key] = self.sanitize_unknown(result[key], ("interaction", key))
        return result

    def sanitize_transcript(self, transcript: dict[str, Any]) -> dict[str, Any]:
        result = copy.deepcopy(transcript)
        if "callId" in result:
            result["callId"] = str(result["callId"])
        for entry in result.get("transcript") or []:
            if entry.get("speakerId") is not None:
                entry["speakerId"] = numeric_identifier("identity-id", entry["speakerId"])
            if isinstance(entry.get("topic"), str):
                entry["topic"] = f"T_{digest('transcript-topic', entry['topic'])}"
            for sentence in entry.get("sentences") or []:
                if isinstance(sentence.get("text"), str):
                    sentence["text"] = f"X_{digest('sentence', sentence['text'])}."
        for key in list(result):
            if key not in {"callId", "transcript"}:
                result[key] = self.sanitize_unknown(result[key], ("callTranscript", key))
        return result

    def sanitize_unknown(self, value: Any, path: tuple[Any, ...]) -> Any:
        if value is None or isinstance(value, (bool, int, float)):
            return value
        if isinstance(value, list):
            return [self.sanitize_unknown(child, path + (index,)) for index, child in enumerate(value)]
        if isinstance(value, dict):
            return {key: self.sanitize_unknown(child, path + (key,)) for key, child in value.items()}
        if not isinstance(value, str):
            raise SanitizationError(f"unsupported JSON value at {path}: {type(value).__name__}")
        if TIMESTAMP.match(value):
            return value
        key = path[-1] if path else "value"
        if key in {"id", "userId", "speakerId", "requestId", "cursor"}:
            return numeric_identifier("identity-id", value)
        if key == "affiliation" and value in {"Internal", "External", "Unknown"}:
            return value
        return f"Z_{digest('unknown-string', value)}"


def collect_denylist(raw_documents: Iterable[dict[str, Any]]) -> set[tuple[str, str]]:
    markers = {("substring", "contrastsecurity.com")}
    for document in raw_documents:
        for _, value in iter_strings(document):
            for match in EMAIL.finditer(value):
                markers.add(("substring", match.group(0)))
                markers.add(("substring", match.group(1)))
            if "$" in value:
                markers.add(("substring", value))

        for call in document.get("calls", []):
            for party in call.get("parties", []) or []:
                name = party.get("name")
                if isinstance(name, str) and name.strip():
                    markers.add(("substring", name.strip()))
                    words = re.findall(r"[A-Za-zÀ-ÖØ-öø-ÿ'’-]+", name)
                    if words:
                        markers.add(("token", words[-1]))
            for context in call.get("context", []) or []:
                for obj in context.get("objects", []) or []:
                    for field in obj.get("fields", []) or []:
                        value = field.get("value")
                        if field.get("name") == "Name" and isinstance(value, str):
                            markers.add(("substring", value))
                        if MONEY_FIELD.search(str(field.get("name", ""))) and isinstance(value, str):
                            markers.add(("substring", value))
    return {
        (kind, marker.casefold())
        for kind, marker in markers
        if marker and len(marker.strip()) >= 2
    }


def is_structural_value(path: tuple[Any, ...], value: str) -> bool:
    if TIMESTAMP.match(value) or re.fullmatch(r"\d{6,}", value):
        return True
    if value in {"Internal", "External", "Unknown", "Account", "CRMObject", "CRM", "Next steps"}:
        return True
    if len(path) >= 2 and path[-1] == "name" and "fields" in path:
        return True
    if path and path[-1] in {"direction", "system", "scope", "media", "language"}:
        return True
    return False


def verify_denylist(
    documents: Iterable[dict[str, Any]], denylist: set[tuple[str, str]]
) -> None:
    leaks: list[str] = []
    for document in documents:
        for path, value in iter_strings(document):
            if is_structural_value(path, value):
                continue
            folded = value.casefold()
            for kind, marker in denylist:
                if kind == "substring":
                    found = marker in folded
                else:
                    found = re.search(
                        rf"(?<![a-z0-9]){re.escape(marker)}(?![a-z0-9])", folded
                    ) is not None
                if found:
                    leaks.append(f"{'.'.join(map(str, path))}: denylisted marker remained")
                    break
    if leaks:
        raise SanitizationError("denylist verification failed:\n" + "\n".join(leaks[:20]))


def all_calls(documents: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    return [call for document in documents for call in document.get("calls", [])]


def local_date(call: dict[str, Any]) -> str:
    return str(call["metaData"]["started"])[:10]


def verify_quirks(documents: list[dict[str, Any]]) -> None:
    calls = all_calls(documents)
    parties = [party for call in calls for party in call.get("parties", []) or []]

    unknown_null = [
        party
        for party in parties
        if party.get("affiliation") == "Unknown"
        and party.get("name") is None
        and party.get("emailAddress") is None
    ]
    if len(unknown_null) < 5:
        raise SanitizationError("quirk 1 missing: Unknown phone Parties with null name/email")
    if not any(party.get("emailAddress") and party.get("name") is None for party in parties):
        raise SanitizationError("quirk 2 missing: Party with email and null name")
    if not any(party.get("name") and party.get("emailAddress") is None for party in parties):
        raise SanitizationError("quirk 2 missing: Party with name and null email")

    missing_spotlight = [
        call
        for call in calls
        if call.get("content") is None
        or (
            isinstance(call.get("content"), dict)
            and not any(key in call["content"] for key in ("brief", "keyPoints", "highlights"))
        )
    ]
    if len(missing_spotlight) < 2:
        raise SanitizationError("quirk 3 missing: two Calls without Spotlight fields")

    call_ids = [str(call.get("metaData", {}).get("id", "")) for call in calls]
    if (
        REFERENCE_CALL_ID not in call_ids
        or not all(re.fullmatch(r"\d{17,19}", call_id) for call_id in call_ids)
        or not any(len(call_id) == 19 for call_id in call_ids)
    ):
        raise SanitizationError("quirk 4 missing: exact string Call IDs and 19-digit reference case")

    same_day = Counter((local_date(call), call["metaData"]["title"]) for call in calls)
    if not any(count > 1 for count in same_day.values()):
        raise SanitizationError("quirk 5 missing: same-day duplicate titles")
    title_dates: defaultdict[str, set[str]] = defaultdict(set)
    for call in calls:
        title_dates[call["metaData"]["title"]].add(local_date(call))
    if not any(len(dates) > 1 for dates in title_dates.values()):
        raise SanitizationError("quirk 5 missing: duplicate title on different dates")

    titles = "".join(call["metaData"]["title"] for call in calls)
    if not all(character in titles for character in '<>:/|“'):
        raise SanitizationError("quirk 6 missing: filename-hostile title character classes")

    reference = next(call for call in calls if call["metaData"]["id"] == REFERENCE_CALL_ID)
    outline = reference.get("content", {}).get("outline", [])
    if len(outline) != 25 or not any(isinstance(item.get("startTime"), float) for item in outline):
        raise SanitizationError("quirk 7 missing: 25-section float-seconds Outline")

    account_objects = [
        obj
        for call in calls
        for context in call.get("context", []) or []
        for obj in context.get("objects", []) or []
        if obj.get("objectType") == "Account"
    ]
    if not account_objects:
        raise SanitizationError("quirk 8 missing: CRM Account context")
    field_names = {field.get("name") for obj in account_objects for field in obj.get("fields", []) or []}
    if "Name" not in field_names or not any("ARR" in str(name) for name in field_names):
        raise SanitizationError("quirk 8 missing: Account Name and deal-field structure")

    transcript_documents = [document for document in documents if "callTranscripts" in document]
    if not transcript_documents:
        raise SanitizationError("transcript fixture missing")
    transcript = transcript_documents[0]["callTranscripts"][0]
    if transcript["callId"] != REFERENCE_CALL_ID:
        raise SanitizationError("transcript no longer joins to reference Call")
    party_speaker_ids = {party.get("speakerId") for party in reference.get("parties", [])}
    transcript_speaker_ids = {item.get("speakerId") for item in transcript.get("transcript", [])}
    if not transcript_speaker_ids <= party_speaker_ids:
        raise SanitizationError("sanitization broke speakerId joins")


def dump_json(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def load_documents(source_dir: Path) -> list[dict[str, Any]]:
    missing = [name for name in FIXTURE_NAMES if not (source_dir / name).is_file()]
    if missing:
        raise SanitizationError(f"missing raw spike files in {source_dir}: {', '.join(missing)}")
    return [json.loads((source_dir / name).read_text(encoding="utf-8")) for name in FIXTURE_NAMES]


def run(source_dir: Path, output_dir: Path, check: bool) -> None:
    raw = load_documents(source_dir)
    sanitizer = Sanitizer()
    sanitized = [sanitizer.sanitize_document(document) for document in raw]
    verify_denylist(sanitized, collect_denylist(raw))
    verify_quirks(sanitized)

    rendered = {name: dump_json(document) for name, document in zip(FIXTURE_NAMES, sanitized, strict=True)}
    if check:
        failures = []
        for name, expected in rendered.items():
            path = output_dir / name
            if not path.is_file() or path.read_bytes() != expected:
                failures.append(name)
        if failures:
            raise SanitizationError("fixtures are absent or stale: " + ", ".join(failures))
        print(f"verified {len(rendered)} deterministic fixtures and all eight quirk classes")
        return

    output_dir.mkdir(parents=True, exist_ok=True)
    for name, contents in rendered.items():
        (output_dir / name).write_bytes(contents)
    print(f"wrote {len(rendered)} sanitized fixtures to {output_dir}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, default=Path("spikes"))
    parser.add_argument("--output-dir", type=Path, default=Path("tests/fixtures"))
    parser.add_argument("--check", action="store_true", help="verify committed fixtures are current")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        run(args.source_dir, args.output_dir, args.check)
    except (OSError, json.JSONDecodeError, SanitizationError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
