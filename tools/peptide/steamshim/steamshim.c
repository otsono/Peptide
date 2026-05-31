// Shim that ACTUALLY initializes Steam from a load constructor (the Steam client
// is running). If the real context comes up, the engine sees [API loaded yes]
// and the real UGC pipeline loads custom content — no faking, no bytecode edits.
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

extern bool SteamAPI_Init(void);                 // legacy bool init
extern int  SteamInternal_SteamAPI_Init(const char*, void*); // modern flat init
extern bool SteamAPI_IsSteamRunning(void);

__attribute__((constructor))
static void shim_init(void){
  // Ensure the appid is visible to the SDK before init.
  setenv("SteamAppId", "1420350", 1);
  setenv("SteamGameId", "1420350", 1);
  fprintf(stderr, "[steamshim] ctor: IsSteamRunning(pre)=%d\n", SteamAPI_IsSteamRunning());
  bool ok = SteamAPI_Init();
  fprintf(stderr, "[steamshim] ctor: SteamAPI_Init()=%d IsSteamRunning(post)=%d\n",
          ok, SteamAPI_IsSteamRunning());
}
