#ifndef DGN_BM_NETDB_H
#define DGN_BM_NETDB_H
#include <sys/socket.h>
struct addrinfo { int ai_flags, ai_family, ai_socktype, ai_protocol; socklen_t ai_addrlen; struct sockaddr* ai_addr; char* ai_canonname; struct addrinfo* ai_next; };
struct hostent { char* h_name; char** h_aliases; int h_addrtype; int h_length; char** h_addr_list; };
struct hostent* gethostbyname(const char*);
int getaddrinfo(const char*, const char*, const struct addrinfo*, struct addrinfo**);
void freeaddrinfo(struct addrinfo*);
#endif
