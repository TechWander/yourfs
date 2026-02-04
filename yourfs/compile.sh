gcc yourfs_tool.c -o yourfs_tool -O2 -D_FILE_OFFSET_BITS=64 -std=gnu99 -Wl,-Bstatic -lfuse -lnfs -lsqlite3 -Wl,-Bdynamic -lpq -lm
gcc yourfs.c      -o yourfs      -O2 -D_FILE_OFFSET_BITS=64 -std=gnu99 -Wl,-Bstatic -lfuse -lnfs -lsqlite3 -Wl,-Bdynamic -lpq -lm
