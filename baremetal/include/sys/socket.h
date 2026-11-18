#ifndef DGN_BM_SOCKET_H
#define DGN_BM_SOCKET_H
#include <stddef.h>
typedef unsigned socklen_t;
struct sockaddr { unsigned short sa_family; char sa_data[14]; };
#define AF_INET 2
#define SOCK_STREAM 1
int socket(int, int, int); int connect(int, const struct sockaddr*, socklen_t);
long send(int, const void*, size_t, int); long recv(int, void*, size_t, int);
int bind(int, const struct sockaddr*, socklen_t); int listen(int, int); int accept(int, struct sockaddr*, socklen_t*);
int setsockopt(int, int, int, const void*, socklen_t);
#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#endif
