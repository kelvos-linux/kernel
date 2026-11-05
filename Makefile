.PHONY: build clean

build:
	./build.sh

clean:
	cargo clean
	rm -rf iso
	rm -f kernel.elf kelv-os.iso target/boot.o
