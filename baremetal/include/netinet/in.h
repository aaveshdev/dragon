#ifndef DGN_BM_NETINET_H
#define DGN_BM_NETINET_H
#include <sys/socket.h>
struct in_addr { unsigned s_addr; };
struct sockaddr_in { unsigned short sin_family; unsigned short sin_port; struct in_addr sin_addr; char sin_zero[8]; };
unsigned short htons(unsigned short); unsigned htonl(unsigned); unsigned short ntohs(unsigned short);
#define INADDR_ANY 0
#endif
