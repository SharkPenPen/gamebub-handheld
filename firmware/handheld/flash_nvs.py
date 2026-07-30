import argparse
import os
import sys
import tempfile
import subprocess

NAMESPACE: str = "gb"
PARTITION_NAME: str = "nvs_ro"

class KvsData:
    lines: list[str]

    def __init__(self):
        self.lines = ["key,type,encoding,value"]

    @staticmethod
    def _encode_varint(value: int) -> bytearray:
        """Encode a varint in Postcard format"""
        data = bytearray()
        while value:
            unit = value & 0x7F
            remainder = value >> 7
            if remainder:
                unit |= 0x80
            data.append(unit)
            value = remainder
        return data

    @staticmethod
    def _encode_bytes(value: bytes) -> bytearray:
        """Encode bytes or a string in Postcard format"""
        
    def _put(self, key: str, encoded: bytes) -> None:
        self.lines.append(f"{key},data,hex2bin,{encoded.hex()}")

    def namespace(self, name: str) -> None:
        self.lines.append(f"{name},namespace,,")

    def put_uint(self, key: str, value: int) -> None:
        data = KvsData._encode_varint(value)
        self._put(key, data)

    def put_string(self, key: str, value: str) -> None:
        self.put_bytes(key, value.encode("utf-8"))

    def put_bytes(self, key: str, value: bytes) -> None:
        data = KvsData._encode_varint(len(value))
        data += value
        self._put(key, data)

    def get(self) -> str:
        return "\n".join(self.lines)

        

def main() -> None:
    esp_idf_path = os.environ.get("IDF_PATH")
    if not esp_idf_path:
        print("IDF_PATH not set: source the esp-idf export.sh")
        sys.exit(1)

    parser = argparse.ArgumentParser()
    parser.add_argument("--serial", required=True)
    parser.add_argument("--revision", required=True, type=int)
    args = parser.parse_args()

    partition_offset = None
    partition_size = None
    with open("partitions.csv") as f:
        for line in f:
            entry = [x.strip() for x in line.split(",")]
            if entry[0] == PARTITION_NAME:
                partition_offset = int(entry[3], base=0)
                partition_size = int(entry[4], base=0)
                print(f"Found partition in table: offset={hex(partition_offset)} size={hex(partition_size)}")
    if partition_offset is None:
        print("Couldn't find partition in partition table")
        sys.exit(1)


    data = KvsData()
    data.namespace(NAMESPACE)
    if args.serial:
        data.put_string("serial", args.serial)
    if args.revision:
        data.put_uint("revision", args.revision)

    in_file = tempfile.NamedTemporaryFile(mode="w", delete_on_close=False, suffix=".csv")
    in_file.write(data.get())
    in_file.close()

    out_file = tempfile.NamedTemporaryFile(delete_on_close=False, suffix=".bin")
    out_file.close()

    script_path = os.path.join(esp_idf_path, "components/nvs_flash/nvs_partition_generator/nvs_partition_gen.py")
    subprocess.run([script_path, "generate", in_file.name, out_file.name, str(partition_size)])

    confirm = input("Flash the read-only NVS partition [y/n]? ")
    if confirm.strip().lower() != "y":
        print("Cancelling")
        return
    
    print("Flashing...")
    subprocess.run(["espflash", "write-bin", str(partition_offset), out_file.name])


if __name__ == "__main__":
    main()
