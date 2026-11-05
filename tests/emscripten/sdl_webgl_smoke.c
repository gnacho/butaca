#include <SDL.h>
#include <SDL_opengles2.h>
#include <emscripten.h>
#include <stdio.h>
#include <string.h>

static SDL_Window *window;
static SDL_GLContext context;
static GLuint program;
static GLuint vertices;
static int reported;

static void report(const char *message, const char *status) {
    EM_ASM({
        let node = document.getElementById('butaca-smoke-result');
        if (!node) {
            node = document.createElement('pre');
            node.id = 'butaca-smoke-result';
            document.body.appendChild(node);
        }
        node.dataset.status = UTF8ToString($1);
        node.textContent = UTF8ToString($0);
    }, message, status);
}

static GLuint compile_shader(GLenum type, const char *source) {
    GLuint shader = glCreateShader(type);
    glShaderSource(shader, 1, &source, NULL);
    glCompileShader(shader);
    GLint compiled = GL_FALSE;
    glGetShaderiv(shader, GL_COMPILE_STATUS, &compiled);
    if (!compiled) {
        char log[1024] = {0};
        glGetShaderInfoLog(shader, sizeof(log) - 1, NULL, log);
        fprintf(stderr, "shader compilation failed: %s\n", log);
        glDeleteShader(shader);
        return 0;
    }
    return shader;
}

static void fail(const char *message) {
    report(message, "fail");
    fprintf(stderr, "%s\n", message);
}

static void frame(void) {
    SDL_Event event;
    while (SDL_PollEvent(&event)) {
        if (event.type == SDL_QUIT) {
            emscripten_cancel_main_loop();
            return;
        }
    }

    int width = 0;
    int height = 0;
    SDL_GL_GetDrawableSize(window, &width, &height);
    glViewport(0, 0, width, height);
    glClearColor(0.04f, 0.08f, 0.14f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);

    const GLfloat points[] = {-0.8f, -0.8f, 0.8f, -0.8f, 0.0f, 0.8f};
    glBindBuffer(GL_ARRAY_BUFFER, vertices);
    glBufferSubData(GL_ARRAY_BUFFER, 0, sizeof(points), points);
    glUseProgram(program);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, (const void *)0);
    glDrawArrays(GL_TRIANGLES, 0, 3);

    GLenum error = glGetError();
    if (error != GL_NO_ERROR) {
        char message[96];
        snprintf(message, sizeof(message), "GLES2 draw failed: glGetError=0x%04x", error);
        fail(message);
        emscripten_cancel_main_loop();
        return;
    }

    SDL_GL_SwapWindow(window);
    if (!reported) {
        reported = 1;
        const GLubyte *version = glGetString(GL_VERSION);
        char message[256];
        snprintf(message, sizeof(message), "SDL GLES2 + WebGL shader/VBO draw passed (%s)",
                 version ? (const char *)version : "no GL_VERSION");
        report(message, "pass");
        printf("%s\n", message);
    }
}

int main(void) {
    if (SDL_Init(SDL_INIT_VIDEO) != 0) {
        fail(SDL_GetError());
        return 1;
    }
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_PROFILE_MASK, SDL_GL_CONTEXT_PROFILE_ES);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MAJOR_VERSION, 2);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MINOR_VERSION, 0);

    window = SDL_CreateWindow("Butaca WebGL smoke", SDL_WINDOWPOS_CENTERED,
                              SDL_WINDOWPOS_CENTERED, 800, 450,
                              SDL_WINDOW_OPENGL | SDL_WINDOW_RESIZABLE);
    if (!window) {
        fail(SDL_GetError());
        return 1;
    }
    context = SDL_GL_CreateContext(window);
    if (!context) {
        fail(SDL_GetError());
        return 1;
    }

    static const char vertex_source[] =
        "attribute vec2 position;\n"
        "void main() { gl_Position = vec4(position, 0.0, 1.0); }\n";
    static const char fragment_source[] =
        "precision mediump float;\n"
        "void main() { gl_FragColor = vec4(0.96, 0.72, 0.24, 1.0); }\n";
    GLuint vs = compile_shader(GL_VERTEX_SHADER, vertex_source);
    GLuint fs = compile_shader(GL_FRAGMENT_SHADER, fragment_source);
    if (!vs || !fs) {
        fail("GLSL ES 1.00 shader compilation failed");
        return 1;
    }
    program = glCreateProgram();
    glAttachShader(program, vs);
    glAttachShader(program, fs);
    glBindAttribLocation(program, 0, "position");
    glLinkProgram(program);
    GLint linked = GL_FALSE;
    glGetProgramiv(program, GL_LINK_STATUS, &linked);
    if (!linked) {
        char log[1024] = {0};
        glGetProgramInfoLog(program, sizeof(log) - 1, NULL, log);
        fprintf(stderr, "program link failed: %s\n", log);
        fail("GLSL program link failed");
        return 1;
    }
    glDeleteShader(vs);
    glDeleteShader(fs);

    glGenBuffers(1, &vertices);
    glBindBuffer(GL_ARRAY_BUFFER, vertices);
    glBufferData(GL_ARRAY_BUFFER, 6 * sizeof(GLfloat), NULL, GL_DYNAMIC_DRAW);
    if (glGetError() != GL_NO_ERROR) {
        fail("VBO allocation failed");
        return 1;
    }

    emscripten_set_main_loop(frame, 0, 1);
    return 0;
}
