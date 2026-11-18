#ifndef DGN_BM_STDIO_H
#define DGN_BM_STDIO_H
#include <stddef.h>
#include <stdarg.h>
typedef struct DgnBmFile FILE;
extern FILE* stdin; extern FILE* stdout; extern FILE* stderr;
#define EOF (-1)
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
int printf(const char*, ...);
int fprintf(FILE*, const char*, ...);
int snprintf(char*, size_t, const char*, ...);
int sprintf(char*, const char*, ...);
int vsnprintf(char*, size_t, const char*, va_list);
int vsprintf(char*, const char*, va_list);
int vfprintf(FILE*, const char*, va_list);
int vprintf(const char*, va_list);
int puts(const char*);
int putchar(int);
int fputs(const char*, FILE*);
int fputc(int, FILE*);
int getchar(void);
int fflush(FILE*);
FILE* fopen(const char*, const char*);
int fclose(FILE*);
size_t fread(void*, size_t, size_t, FILE*);
size_t fwrite(const void*, size_t, size_t, FILE*);
int fseek(FILE*, long, int);
long ftell(FILE*);
void rewind(FILE*);
char* fgets(char*, int, FILE*);
int fgetc(FILE*);
int feof(FILE*);
int remove(const char*);
int rename(const char*, const char*);
int sscanf(const char*, const char*, ...);
#endif
