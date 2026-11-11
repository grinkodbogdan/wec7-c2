/*
 * OpenGL smoke test: Mesa's software renderer (OSMesa + softpipe) draws
 * a shaded triangle with a GLSL program; check pixels inside and outside.
 */
#define GL_GLEXT_PROTOTYPES
#include <GL/osmesa.h>
#include <GL/gl.h>
#include <GL/glext.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#define W 256
#define H 256

static const char *vs =
    "#version 120\n"
    "attribute vec2 pos;\n"
    "void main() { gl_Position = vec4(pos, 0.0, 1.0); }\n";
static const char *fs =
    "#version 120\n"
    "void main() { gl_FragColor = vec4(1.0, 0.5, 0.0, 1.0); }\n";

static GLuint
shader(GLenum type, const char *src)
{
	GLuint s = glCreateShader(type);
	GLint ok;
	char log[512];

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	if (!ok) {
		glGetShaderInfoLog(s, sizeof(log), NULL, log);
		printf("shader: %s\n", log);
		exit(1);
	}
	return (s);
}

int
main(void)
{
	static const GLfloat tri[] = { -0.8f, -0.8f, 0.8f, -0.8f, 0.0f, 0.8f };
	unsigned char *buf = calloc(W * H, 4), *in, *out;
	OSMesaContext ctx;
	GLuint prog;
	struct timespec t0, t1;
	int fails = 0, i;

	ctx = OSMesaCreateContextExt(OSMESA_RGBA, 24, 0, 0, NULL);
	if (ctx == NULL || !OSMesaMakeCurrent(ctx, buf, GL_UNSIGNED_BYTE, W, H)) {
		printf("gl: no OSMesa context\n");
		return (1);
	}
	printf("  GL_RENDERER %s\n  GL_VERSION  %s\n  GLSL        %s\n",
	    glGetString(GL_RENDERER), glGetString(GL_VERSION),
	    glGetString(GL_SHADING_LANGUAGE_VERSION));

	prog = glCreateProgram();
	glAttachShader(prog, shader(GL_VERTEX_SHADER, vs));
	glAttachShader(prog, shader(GL_FRAGMENT_SHADER, fs));
	glBindAttribLocation(prog, 0, "pos");
	glLinkProgram(prog);
	glUseProgram(prog);
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, tri);
	glEnableVertexAttribArray(0);

	clock_gettime(CLOCK_MONOTONIC, &t0);
	for (i = 0; i < 20; i++) {
		glClearColor(0.0f, 0.0f, 0.5f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
	}
	glFinish();
	clock_gettime(CLOCK_MONOTONIC, &t1);

	in = buf + (H / 2 * W + W / 2) * 4;	/* triangle middle */
	out = buf + (2 * W + 2) * 4;		/* bottom-left corner */
	printf("  centre %02x%02x%02x, corner %02x%02x%02x\n",
	    in[0], in[1], in[2], out[0], out[1], out[2]);
	fails += !(in[0] == 0xff && in[1] >= 0x7f && in[1] <= 0x80 && in[2] == 0);
	fails += !(out[0] == 0 && out[1] == 0 && out[2] >= 0x7f && out[2] <= 0x80);
	printf("  20 frames of %dx%d in %ld ms\n", W, H,
	    (long)((t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000));
	printf("%s\n", fails ? "gl: SOME CHECKS FAILED" : "gl: all checks passed");
	return (fails);
}
