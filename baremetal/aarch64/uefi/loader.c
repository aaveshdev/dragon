/* UEFI loader for the AArch64 baremetal kernel: loads \vaelis.elf from the boot volume, finds the screen (GOP),
 * the ACPI tables and the free memory, leaves boot services and starts the kernel with a boot information block in x0. */
typedef unsigned long long u64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;
typedef u64 UINTN;
typedef u64 EFI_STATUS;
typedef void* EFI_HANDLE;
typedef u16 CHAR16;
#define EFIERR(n) ((1ULL << 63) | (n))
#define NULL ((void*)0)

typedef struct { u32 a; u16 b, c; u8 d[8]; } GUID;
typedef struct { u64 sig; u32 rev, size, crc, res; } TableHdr;

typedef struct {
    u32 Type; u32 pad; u64 PhysicalStart, VirtualStart, NumberOfPages, Attribute;
} MemDesc;

typedef struct {
    TableHdr hdr;
    void *RaiseTPL, *RestoreTPL;
    EFI_STATUS (*AllocatePages)(u32 type, u32 memtype, UINTN pages, u64* addr);
    EFI_STATUS (*FreePages)(u64 addr, UINTN pages);
    EFI_STATUS (*GetMemoryMap)(UINTN* size, MemDesc* map, UINTN* key, UINTN* dsize, u32* dver);
    EFI_STATUS (*AllocatePool)(u32 type, UINTN size, void** buf);
    EFI_STATUS (*FreePool)(void* buf);
    void *CreateEvent, *SetTimer, *WaitForEvent, *SignalEvent, *CloseEvent, *CheckEvent;
    void *InstallProtocolInterface, *ReinstallProtocolInterface, *UninstallProtocolInterface;
    EFI_STATUS (*HandleProtocol)(EFI_HANDLE h, const GUID* g, void** iface);
    void *Reserved, *RegisterProtocolNotify, *LocateHandle, *LocateDevicePath, *InstallConfigurationTable;
    void *LoadImage, *StartImage, *Exit, *UnloadImage;
    EFI_STATUS (*ExitBootServices)(EFI_HANDLE img, UINTN key);
    void* GetNextMonotonicCount;
    EFI_STATUS (*Stall)(UINTN us);
    EFI_STATUS (*SetWatchdogTimer)(UINTN timeout, u64 code, UINTN dsize, CHAR16* data);
    void *ConnectController, *DisconnectController, *OpenProtocol, *CloseProtocol, *OpenProtocolInformation, *ProtocolsPerHandle, *LocateHandleBuffer;
    EFI_STATUS (*LocateProtocol)(const GUID* g, void* reg, void** iface);
} BootServices;

typedef struct SimpleTextOut {
    void* Reset;
    EFI_STATUS (*OutputString)(struct SimpleTextOut* self, CHAR16* s);
} SimpleTextOut;

typedef struct { GUID g; void* table; } ConfTable;
typedef struct {
    TableHdr hdr;
    CHAR16* vendor; u32 rev, pad;
    void *ConInHandle, *ConIn, *ConOutHandle; SimpleTextOut* ConOut; void *StdErrHandle, *StdErr, *Runtime;
    BootServices* BS; UINTN nconf; ConfTable* conf;
} SystemTable;

typedef struct {
    u32 Version, HRes, VRes, PixelFormat; u32 mask[4]; u32 PixelsPerScanLine;
} GopInfo;
typedef struct { u32 MaxMode, Mode; GopInfo* Info; UINTN InfoSize; u64 FbBase; UINTN FbSize; } GopMode;
typedef struct Gop {
    EFI_STATUS (*QueryMode)(struct Gop*, u32 n, UINTN* size, GopInfo** info);
    EFI_STATUS (*SetMode)(struct Gop*, u32 n);
    void* Blt; GopMode* Mode;
} Gop;

typedef struct {
    u32 Rev; u32 pad; EFI_HANDLE Parent; SystemTable* ST; EFI_HANDLE DeviceHandle;
} LoadedImage;

typedef struct EFile {
    u64 Rev;
    EFI_STATUS (*Open)(struct EFile*, struct EFile** out, CHAR16* name, u64 mode, u64 attr);
    EFI_STATUS (*Close)(struct EFile*);
    void* Delete;
    EFI_STATUS (*Read)(struct EFile*, UINTN* size, void* buf);
    void* Write;
    void* GetPosition;
    EFI_STATUS (*SetPosition)(struct EFile*, u64 pos);
} EFile;
typedef struct SFS { u64 Rev; EFI_STATUS (*OpenVolume)(struct SFS*, EFile** root); } SFS;

typedef struct {
    u64 magic, version;
    u64 fb_base, fb_w, fb_h, fb_stride, fb_fmt;
    u64 heap_lo, heap_hi, rsdp, uart_base, flags;
    u64 normal_gb[16];
} A64Boot;

static const GUID g_gop = { 0x9042a9de, 0x23dc, 0x4a38, { 0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a } };
static const GUID g_loaded = { 0x5b1b31a1, 0x9562, 0x11d2, { 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b } };
static const GUID g_sfs = { 0x964e5b22, 0x6459, 0x11d2, { 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b } };
static const GUID g_acpi2 = { 0x8868e871, 0xe4f1, 0x11d3, { 0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x80, 0x81 } };

static SystemTable* ST;
static BootServices* BS;

static void say(const char* s) {
    CHAR16 buf[160]; int n = 0;
    while (*s && n < 156) { if (*s == '\n') buf[n++] = '\r'; buf[n++] = (CHAR16)(u8)*s++; }
    buf[n] = 0;
    if (ST && ST->ConOut) ST->ConOut->OutputString(ST->ConOut, buf);
}
static void say_hex(u64 v) {
    char b[20]; for (int i = 0; i < 16; i++) { int d = (int)((v >> (60 - 4 * i)) & 15); b[i] = (char)(d < 10 ? '0' + d : 'a' + d - 10); } b[16] = 0; say(b);
}
static int guid_eq(const GUID* a, const GUID* b) {
    const u64* x = (const u64*)a; const u64* y = (const u64*)b; return x[0] == y[0] && x[1] == y[1];
}
static void* mset(void* d, int c, u64 n) { u8* p = (u8*)d; while (n--) *p++ = (u8)c; return d; }
void* memset(void* d, int c, unsigned long n) { return mset(d, c, n); }
void* memcpy(void* d, const void* s, unsigned long n) { u8* a = (u8*)d; const u8* b = (const u8*)s; while (n--) *a++ = *b++; return d; }

static void clean_range(u64 a, u64 n) {
    for (u64 p = a & ~63ULL; p < a + n; p += 64) __asm__ volatile("dc civac, %0" :: "r"(p) : "memory");
}

static EFI_STATUS fail(const char* m, EFI_STATUS st) { say("vaelis: "); say(m); say(" ("); say_hex(st); say(")\n"); BS->Stall(8000000); return st; }

typedef struct { u8 id[16]; u16 type, machine; u32 ver; u64 entry, phoff, shoff; u32 flags; u16 ehsize, phentsize, phnum, shentsize, shnum, shstrndx; } Ehdr;
typedef struct { u32 type, flags; u64 off, vaddr, paddr, filesz, memsz, align; } Phdr;

static void mark_gb(A64Boot* bi, u64 start, u64 end) {
    if (end <= start) return;
    for (u64 g = start >> 30; g <= (end - 1) >> 30 && g < 1024; g++) bi->normal_gb[g >> 6] |= 1ULL << (g & 63);
}

static MemDesc* mapbuf; static UINTN mapcap;
static EFI_STATUS get_map(UINTN* size, UINTN* key, UINTN* dsize) {
    u32 ver; *size = mapcap;
    return BS->GetMemoryMap(size, mapbuf, key, dsize, &ver);
}

EFI_STATUS efi_main(EFI_HANDLE image, SystemTable* st) {
    ST = st; BS = st->BS;
    BS->SetWatchdogTimer(0, 0, 0, NULL);
    say("Vaelis ARM64 loader\n");

    A64Boot* bi; if (BS->AllocatePool(2, sizeof(A64Boot) + 4096, (void**)&bi)) return fail("no memory", 0);
    mset(bi, 0, sizeof *bi);
    bi->magic = 0x5641454C49534249ULL; bi->version = 1;

    /* screen */
    Gop* gop = NULL;
    if (BS->LocateProtocol(&g_gop, NULL, (void**)&gop) || !gop) return fail("no graphics output (GOP)", 1);
    {   /* prefer 1280x720 when the firmware offers it, otherwise keep what it chose */
        for (u32 m = 0; m < gop->Mode->MaxMode; m++) {
            UINTN sz; GopInfo* in;
            if (gop->QueryMode(gop, m, &sz, &in)) continue;
            if (in->HRes == 1280 && in->VRes == 720 && in->PixelFormat < 2) { if (gop->Mode->Mode != m) gop->SetMode(gop, m); break; }
        }
    }
    GopInfo* gi = gop->Mode->Info;
    if (gi->PixelFormat > 1 && gi->PixelFormat != 2) return fail("the screen has no linear framebuffer", gi->PixelFormat);
    bi->fb_base = gop->Mode->FbBase; bi->fb_w = gi->HRes; bi->fb_h = gi->VRes; bi->fb_stride = gi->PixelsPerScanLine;
    bi->fb_fmt = gi->PixelFormat == 0 ? 0 : 1;
    if (gi->PixelFormat == 2) bi->fb_fmt = (gi->mask[0] == 0xff) ? 0 : 1;

    /* ACPI */
    for (UINTN i = 0; i < st->nconf; i++) if (guid_eq(&st->conf[i].g, &g_acpi2)) bi->rsdp = (u64)st->conf[i].table;

    /* kernel */
    LoadedImage* li; SFS* sfs; EFile *root, *f;
    if (BS->HandleProtocol(image, &g_loaded, (void**)&li)) return fail("no loaded image", 2);
    if (BS->HandleProtocol(li->DeviceHandle, &g_sfs, (void**)&sfs)) return fail("boot volume has no file system", 3);
    if (sfs->OpenVolume(sfs, &root)) return fail("cannot open the boot volume", 4);
    static CHAR16 name[] = { '\\', 'v', 'a', 'e', 'l', 'i', 's', '.', 'e', 'l', 'f', 0 };
    if (root->Open(root, &f, name, 1, 0)) return fail("\\vaelis.elf not found", 5);
    static u8 hdr[4096]; UINTN n = sizeof hdr;
    if (f->Read(f, &n, hdr) || n < 64) return fail("cannot read the kernel", 6);
    Ehdr* eh = (Ehdr*)hdr;
    if (eh->id[0] != 0x7f || eh->id[1] != 'E' || eh->machine != 183) return fail("the kernel is not an AArch64 ELF", 7);
    Phdr* ph = (Phdr*)(hdr + eh->phoff);
    u64 lo = ~0ULL, hi = 0;
    for (int i = 0; i < eh->phnum; i++) if (ph[i].type == 1) { if (ph[i].paddr < lo) lo = ph[i].paddr; if (ph[i].paddr + ph[i].memsz > hi) hi = ph[i].paddr + ph[i].memsz; }
    lo &= ~4095ULL; hi = (hi + 4095) & ~4095ULL;
    u64 span = hi - lo, align = 0x10000, dynoff = 0, dynsz = 0, base = 0;
    for (int i = 0; i < eh->phnum; i++) { if (ph[i].type == 1 && ph[i].align > align) align = ph[i].align; if (ph[i].type == 2) { dynoff = ph[i].paddr; dynsz = ph[i].memsz; } }
    int pie = eh->type == 3;
    u64 pages = (span + align) >> 12;
    if (pie) {
        u64 a;
        if (BS->AllocatePages(0, 2, pages, &a)) return fail("not enough free memory for the kernel", 8);
        base = ((a + align - 1) & ~(align - 1)) - lo;
    } else {
        base = 0; u64 a = lo;
        if (BS->AllocatePages(2, 2, span >> 12, &a)) return fail("the memory the kernel is linked at is not free", 8);
    }
    u64 kstart = lo + base, kend = hi + base;
    mset((void*)kstart, 0, span);
    for (int i = 0; i < eh->phnum; i++) {
        if (ph[i].type != 1 || !ph[i].filesz) continue;
        if (f->SetPosition(f, ph[i].off)) return fail("cannot seek in the kernel", 9);
        UINTN len = ph[i].filesz;
        if (f->Read(f, &len, (void*)(ph[i].paddr + base)) || len != ph[i].filesz) return fail("cannot read the kernel", 10);
    }
    if (pie) {
        u64 rela = 0, relasz = 0;
        for (u64* d = (u64*)(dynoff + base); d < (u64*)(dynoff + base + dynsz); d += 2) { if (d[0] == 7) rela = d[1]; else if (d[0] == 8) relasz = d[1]; else if (d[0] == 0) break; }
        for (u64 i = 0; i < relasz; i += 24) {
            u64* r = (u64*)(rela + base + i);
            if ((r[1] & 0xffffffff) != 1027) return fail("unsupported relocation in the kernel", r[1] & 0xffffffff);
            *(u64*)(r[0] + base) = base + r[2];
        }
    }
    lo = kstart; hi = kend;
    f->Close(f);
    u64 entry = eh->entry + base;

    /* the biggest block of free memory becomes the kernel heap */
    UINTN size, key, dsize;
    mapcap = 128 * 1024;
    if (BS->AllocatePool(2, mapcap, (void**)&mapbuf)) return fail("no memory for the memory map", 11);
    if (get_map(&size, &key, &dsize)) return fail("cannot read the memory map", 12);
    u64 best = 0, bestn = 0;
    for (u8* p = (u8*)mapbuf; p < (u8*)mapbuf + size; p += dsize) {
        MemDesc* d = (MemDesc*)p;
        if (d->Type == 7 && d->NumberOfPages > bestn) { best = d->PhysicalStart; bestn = d->NumberOfPages; }
    }
    if (bestn < 8192) return fail("less than 32 MB of free memory", 13);
    u64 take = bestn - 2048;
    u64 hbase = best;
    if (BS->AllocatePages(2, 2, take, &hbase)) return fail("cannot reserve the heap", 14);
    bi->heap_lo = best; bi->heap_hi = best + (take << 12);

    clean_range(lo, hi - lo);
    clean_range((u64)bi, sizeof *bi);

    /* leave boot services: the key must come from the very last memory map */
    for (int tries = 0; tries < 4; tries++) {
        if (get_map(&size, &key, &dsize)) return fail("cannot read the memory map", 15);
        if (!BS->ExitBootServices(image, key)) goto left;
    }
    return fail("cannot leave boot services", 16);
left:
    for (u8* p = (u8*)mapbuf; p < (u8*)mapbuf + size; p += dsize) {
        MemDesc* d = (MemDesc*)p;
        if (d->Type >= 1 && d->Type <= 10 && d->Type != 8) mark_gb(bi, d->PhysicalStart, d->PhysicalStart + (d->NumberOfPages << 12));
    }
    mark_gb(bi, kstart, kend);
    mark_gb(bi, bi->heap_lo, bi->heap_hi);
    clean_range((u64)bi, sizeof *bi);
    __asm__ volatile("dsb sy; ic iallu; dsb sy; isb" ::: "memory");
    __asm__ volatile("mov x0, %0; br %1" :: "r"(bi), "r"(entry) : "x0", "memory");
    for (;;) {}
}
