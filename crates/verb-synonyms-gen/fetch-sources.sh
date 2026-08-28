#!/bin/sh
# Download the two lexical sources `verb-synonyms-gen` reads, into $1.
#
# Neither is vendored into the repository: they are large, and their licences
# are satisfied by reproducing a notice (see THIRD-PARTY-NOTICES.md) rather than
# by redistributing the corpora. The digests below pin the exact releases the
# committed table was built from — if one stops matching, the source changed and
# the table's provenance line has to change with it.
set -eu

dest="${1:?usage: fetch-sources.sh <directory>}"
mkdir -p "$dest"
cd "$dest"

wordnet_sha=640db279c949a88f61f851dd54ebbb22d003f8b90b85267042ef85a3781d3a52
dicts_sha=64ac1d35acb66b550c7ebc56e080b62e0bad8f5984d72059dc2e05ac48780e52

check() {
    have=$(shasum -a 256 "$1" | cut -d' ' -f1)
    [ "$have" = "$2" ] || { echo "$1: sha256 $have, expected $2" >&2; exit 1; }
}

# WordNet 3.0. WNdb-3.0.tar.gz is the database alone (index.*, data.*) and does
# NOT carry verb.exc, which the generator needs to tell an irregular inflection
# from a lemma; the full WordNet-3.0.tar.gz does, and also carries the LICENSE.
[ -f WordNet-3.0.tar.gz ] ||
    curl -fsSL -o WordNet-3.0.tar.gz https://wordnetcode.princeton.edu/3.0/WordNet-3.0.tar.gz
check WordNet-3.0.tar.gz "$wordnet_sha"
tar xzf WordNet-3.0.tar.gz WordNet-3.0/dict WordNet-3.0/LICENSE

# 12dicts 6.0.2.
[ -f 12dicts-6.0.2.zip ] ||
    curl -fsSL -o 12dicts-6.0.2.zip \
        "http://downloads.sourceforge.net/wordlist/12dicts-6.0.2.zip"
check 12dicts-6.0.2.zip "$dicts_sha"
unzip -oq 12dicts-6.0.2.zip -d 12dicts

echo "dict:  $dest/WordNet-3.0/dict"
echo "freq:  $dest/12dicts/Lemmatized/2+2+3frq.txt"
