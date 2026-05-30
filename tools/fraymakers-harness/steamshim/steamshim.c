// x86_64 DYLD interpose shim for Fraymakers headless launch.
// Goal: make the engine believe Steam is live so the UGC pipeline runs,
// without the Steam client driving our process. We force the "is Steam
// running / init succeeded" answers to positive. All other Steam calls fall
// through to the real libsteam_api.dylib (we only override a few).
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

// ---- overrides ----
static bool my_SteamAPI_IsSteamRunning(void){ return true; }

// Modern flat init: SteamAPIInitResult SteamInternal_SteamAPI_Init(const char*, void*)
// SteamAPIInitResult: 0 = OK. Force OK.
static int my_SteamInternal_SteamAPI_Init(const char* ver, void* msg){
    (void)ver; (void)msg;
    fprintf(stderr, "[steamshim] SteamInternal_SteamAPI_Init forced OK\n");
    return 0; // k_ESteamAPIInitResult_OK
}

// RestartAppIfNecessary must return false (don't relaunch via Steam).
static bool my_SteamAPI_RestartAppIfNecessary(uint32_t appid){ (void)appid; return false; }

// ---- interpose table ----
#define INTERPOSE(newf, oldf) \
  __attribute__((used)) static struct { const void* n; const void* o; } \
  _interpose_##oldf __attribute__((section("__DATA,__interpose"))) = \
  { (const void*)(unsigned long)&newf, (const void*)(unsigned long)&oldf }

extern bool SteamAPI_IsSteamRunning(void);
extern int  SteamInternal_SteamAPI_Init(const char*, void*);
extern bool SteamAPI_RestartAppIfNecessary(uint32_t);

INTERPOSE(my_SteamAPI_IsSteamRunning, SteamAPI_IsSteamRunning);
INTERPOSE(my_SteamInternal_SteamAPI_Init, SteamInternal_SteamAPI_Init);
INTERPOSE(my_SteamAPI_RestartAppIfNecessary, SteamAPI_RestartAppIfNecessary);
