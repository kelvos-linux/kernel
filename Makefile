.PHONY: build run qemu iso clean

build:
	./build.sh

run:
	./run.sh

qemu: run

iso:
	sh ./make-iso.sh

clean:
	cargo clean
	rm -rf iso kernel.elf kelv-os.iso
