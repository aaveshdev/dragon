#ifndef DGN_BM_MATH_H
#define DGN_BM_MATH_H
double floor(double); double ceil(double); double fabs(double); double sqrt(double);
double fmod(double, double); double pow(double, double); double exp(double); double log(double);
double log10(double); double log2(double); double sin(double); double cos(double); double tan(double);
double asin(double); double acos(double); double atan(double); double atan2(double, double);
double sinh(double); double cosh(double); double tanh(double); double round(double); double trunc(double);
double hypot(double, double); double cbrt(double); double fmin(double, double); double fmax(double, double);
long long llround(double); long lround(double); double rint(double); double ldexp(double,int); double frexp(double,int*); double modf(double,double*);
int isnan(double); int isinf(double);
#define NAN (__builtin_nan(""))
#define INFINITY (__builtin_inf())
#define HUGE_VAL (__builtin_huge_val())
#define M_PI 3.14159265358979323846
#define M_E 2.71828182845904523536
#undef isnan
#undef isinf
#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)
#endif
