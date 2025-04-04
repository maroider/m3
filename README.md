# m3 - maroider's mod manager

A very crude Linux-only "mod manager".

It works by mounting a custom FUSE file-system on top of a game's directory
when the game is launched. All the games files are visible as normal, but
additional files from mods can now be added _virtually_, while retaining a
"clean" game install at the root of it all.

The basic idea can be demonstrated with the file structure in `testdir/`, where
we have the folders `gamedir/` and `moddir/`.

```
gamedir/
├─ gamefile1.txt
└─ subdir/
   ├─ gamefile2.txt
   └─ subsubdir/
      └─ gamefile3.txt
```

```
moddir/
├─ modfile1.txt
└─ subdir/
   └─ modfile2.txt
```

`m3` can then "overlay" the `moddir` on top of `gamedir`, giving us

```
gamedir/
├─ gamefile1.txt
├─ modfile1.txt
└─ subdir/
   ├─ gamefile2.txt
   ├─ modfile2.txt
   └─ subsubdir/
      └─ gamefile3.txt
```

This effect could also be achieved by directly using `overlayfs` rather than
implementing our own poor man's `overlayfs`, but I just felt like using FUSE
directly.

