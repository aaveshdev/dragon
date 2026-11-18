#ifndef DGN_BM_DIRENT_H
#define DGN_BM_DIRENT_H
typedef struct DgnBmDir DIR;
struct dirent { char d_name[256]; };
DIR* opendir(const char*); struct dirent* readdir(DIR*); int closedir(DIR*);
#endif
